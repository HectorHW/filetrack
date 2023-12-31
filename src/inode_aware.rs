use std::{
    cmp::Ordering,
    fs::File,
    io::{self, BufReader},
    ops::{Deref, DerefMut},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{path_utils::glob_rotated_logs, Multireader};

/// Structure that can be used as persistent offset into rotated logs. See `InodeAwareReader` for more info.
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct InodeAwareOffset {
    pub inode: u64,
    pub offset: u64,
}

/// Reader that keeps track of what inode it reads from.
///
/// This reader supports persistent indexing using `InodeAwareOffset`. It allows easy persistent reading of rotated logs.
/// Scheme of persistent is to be implemented by user. For a ready-to-use recipe with simple file storage see `TrackedReader`.
///
/// ```rust no_run
/// # use std::io::{Read, BufRead, self};
/// # use filetrack::{InodeAwareOffset, InodeAwareReader};
/// # fn load_state() -> io::Result<InodeAwareOffset> {Ok(InodeAwareOffset{inode: 0, offset: 0})}
/// # fn save_state(state: InodeAwareOffset) -> io::Result<()> {Ok(())}
/// let mut reader = InodeAwareReader::from_rotated_logs("/var/log/mail.log")?;
/// reader.seek_persistent(load_state()?)?;
/// # let mut buf = vec![];
/// reader.read_exact(& mut buf)?;
/// save_state(reader.get_persistent_offset())?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// During initialization, this reader searches for rotated versions of provided path and notes their inodes. After that inodes can be
/// used for simple persistent indexing when combined with local offset.
pub struct InodeAwareReader {
    inner: Multireader<BufReader<File>>,
    inodes: Vec<u64>,
}

impl InodeAwareReader {
    /// Construct `InodeAwareMultireader` searching for up to two rotated logs.
    pub fn from_rotated_logs(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_rotated_logs_with_depth(path, 2)
    }

    /// Construct `InodeAwareMultireader` searching for up to `max_depth` rotated logs.
    pub fn from_rotated_logs_with_depth(
        path: impl AsRef<Path>,
        max_depth: usize,
    ) -> io::Result<Self> {
        let paths_and_inodes = glob_rotated_logs(path, max_depth)?;
        let (paths, inodes): (Vec<_>, Vec<_>) = paths_and_inodes.into_iter().unzip();
        let files = paths
            .into_iter()
            .map(|path| -> io::Result<BufReader<File>> { Ok(BufReader::new(File::open(path)?)) })
            .collect::<io::Result<Vec<BufReader<File>>>>()?;
        let multireader = Multireader::new(files)?;

        Ok(Self {
            inner: multireader,
            inodes,
        })
    }

    /// Get offset that can be used across restarts and log rotations.
    pub fn get_persistent_offset(&self) -> InodeAwareOffset {
        let inode = self.get_current_inode();
        let offset = self.get_local_offset();
        InodeAwareOffset { inode, offset }
    }

    /// Seek by persistent offset.
    ///
    /// Will return NotFound io error if file with given inode was not found.
    pub fn seek_persistent(&mut self, offset: InodeAwareOffset) -> io::Result<()> {
        let Some(inode_index) = self.get_item_index_by_inode(offset.inode) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "provided inode does not exist",
            ));
        };
        self.seek_by_local_index(inode_index, io::SeekFrom::Start(offset.offset))?;
        Ok(())
    }

    /// Get slice of inodes for current execution.
    pub fn get_inodes(&self) -> &[u64] {
        &self.inodes
    }

    // Destroy struct and return underlying reader and inodes.
    pub fn into_inner(self) -> (Multireader<BufReader<File>>, Vec<u64>) {
        (self.inner, self.inodes)
    }

    /// Get inode of an item that is currently read.
    pub fn get_current_inode(&self) -> u64 {
        let item_index = self.get_current_item_index();
        self.inodes[item_index]
    }

    /// Search for item index by given inode.
    pub fn get_item_index_by_inode(&self, inode: u64) -> Option<usize> {
        self.get_inodes()
            .iter()
            .cloned()
            .enumerate()
            .find(|&(_, i)| i == inode)
            .map(|(idx, _)| idx)
    }

    /// Compare two offsets as if they were pointing into one large buffer. Returns None if any of the offsets do not belong
    /// to underlying files.
    pub fn compare_offsets(
        &self,
        first: InodeAwareOffset,
        second: InodeAwareOffset,
    ) -> Option<Ordering> {
        let first_index = self.get_item_index_by_inode(first.inode)?;
        let second_index = self.get_item_index_by_inode(second.inode)?;
        if first_index == second_index {
            return Some(first.offset.cmp(&second.offset));
        }
        Some(first_index.cmp(&second_index))
    }
}

impl Deref for InodeAwareReader {
    type Target = Multireader<BufReader<File>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for InodeAwareReader {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
