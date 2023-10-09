use std::io::{self, BufRead, Read, Seek};

/// Structure that provides seeking and reading in a sequence of underlying readables
///
/// ## Usage
///
/// Create a Multireader from a collection of items. Items are required to implement `Seek`, and must additionally support Read
/// and BufRead to provide such functionality on resulting aggregate.
///
/// ```rust
/// # use std::io::{Cursor, Read};
/// # use filetrack::Multireader;
/// let inner_items = vec![Cursor::new(vec![1, 2, 3]), Cursor::new(vec![4, 5])];
/// // we get result here because Multireader performs seek
/// // (fallible operation) under the hood to determine sizes
/// let mut reader = Multireader::new(inner_items)?;
/// # let mut buf = vec![];
/// reader.read_to_end(&mut buf)?;
/// assert_eq!(buf, vec![1, 2, 3, 4, 5])
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Multireader allows seeking inside multiple underlying items as if you only had one big buffer
/// ```rust
/// # use std::io::{Cursor, Read, SeekFrom, Seek};
/// # use filetrack::Multireader;
/// # let inner_items = vec![Cursor::new(vec![1, 2, 3]), Cursor::new(vec![4, 5])];
/// # let mut reader = Multireader::new(inner_items)?;
/// reader.seek(SeekFrom::Start(3))?;
/// assert_eq!(reader.get_global_offset(), 3);
/// // you can get index of current item as well as offset into it
/// assert_eq!(reader.get_current_item_index(), 1);
/// assert_eq!(reader.get_local_offset(), 0);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Multireader<R: Seek> {
    /// nonempty
    items: Vec<R>,
    offsets: Vec<u64>,
    global_offset: u64,
}

impl<R: Seek> Multireader<R> {
    /// Create a Multireader from a nonempty collection of readers
    ///
    /// This function returns io::Result because it will use seek to determine sizes which can fail
    pub fn new(mut items: Vec<R>) -> io::Result<Self> {
        assert_ne!(
            items.len(),
            0,
            "you should provide at least one item to be used"
        );
        let sizes = get_sizes_fallible(&mut items)?;
        let offsets = produce_total_offsets(sizes);
        let global_offset = 0;

        Ok(Self {
            items,
            offsets,
            global_offset,
        })
    }

    /// Offset amoung all underlying items
    pub fn get_global_offset(&self) -> u64 {
        self.global_offset
    }

    /// Offset inside current item
    pub fn get_local_offset(&self) -> u64 {
        let item_index = self.get_current_item_index();
        if item_index == 0 {
            return self.global_offset;
        }
        self.global_offset - self.offsets[item_index - 1]
    }

    //we do not have is_empty because, well, this reader cannot be empty
    #[allow(clippy::len_without_is_empty)]
    /// Number of underlying items
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// index of an item that is currently read
    pub fn get_current_item_index(&self) -> usize {
        let mut rightmost_index = 0;
        for &item in &self.offsets {
            if self.global_offset >= item {
                rightmost_index += 1;
            } else {
                break;
            }
        }
        rightmost_index
    }

    /// Destroy the struct and return underlying readers
    pub fn into_inner(self) -> Vec<R> {
        self.items
    }

    /// Get total size of underlying items
    ///
    /// Computes total size of underlying items. This method requires mut ref and returns io::Result
    /// because we need to seek inside last item to determine its size at the moment of call
    pub fn get_total_size(&mut self) -> io::Result<u64> {
        let pre_last_total = self.offsets.last().cloned().unwrap_or_default();
        let last = self.get_last_item_size()?;
        Ok(pre_last_total + last)
    }

    fn get_current_item(&mut self) -> &mut R {
        let index = self.get_current_item_index();
        &mut self.items[index]
    }

    fn get_last_item_size(&mut self) -> io::Result<u64> {
        let last_item = self.items.last_mut().unwrap();
        let original_offset = last_item.stream_position()?;
        let size = last_item.seek(io::SeekFrom::End(0))?;
        last_item.seek(io::SeekFrom::Start(original_offset))?;
        Ok(size)
    }
}

fn produce_total_offsets(mut items: Vec<u64>) -> Vec<u64> {
    let mut total = 0;
    for item in &mut items {
        total += *item;
        *item = total;
    }
    items
}

fn get_sizes_fallible(items: &mut [impl Seek]) -> io::Result<Vec<u64>> {
    let mut offsets = items
        .iter_mut()
        .map(|seekable| -> io::Result<u64> {
            let item_size = seekable.seek(io::SeekFrom::End(0))?;
            seekable.seek(io::SeekFrom::Start(0))?;
            Ok(item_size)
        })
        .collect::<io::Result<Vec<u64>>>()?;
    // last item is ignored
    offsets.pop();

    Ok(offsets)
}

impl<R: Read + Seek> Read for Multireader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let size_read = self.get_current_item().read(buf)?;
        self.global_offset += size_read as u64;
        Ok(size_read)
    }
}

impl<R: BufRead + Seek> BufRead for Multireader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.get_current_item().fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.get_current_item().consume(amt);
        self.global_offset += amt as u64;
    }
}

impl<R: Seek> Seek for Multireader<R> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match pos {
            io::SeekFrom::Start(offset) => {
                self.global_offset = offset;
                let item_index = self.get_current_item_index();
                for item_idx in 0..item_index {
                    self.items[item_idx].seek(io::SeekFrom::End(0))?;
                }
                let local_offset = self.get_local_offset();
                self.get_current_item()
                    .seek(io::SeekFrom::Start(local_offset))?;
                for item_idx in item_index + 1..self.items.len() {
                    self.items[item_idx].seek(io::SeekFrom::Start(0))?;
                }

                Ok(self.global_offset)
            }
            io::SeekFrom::End(offset) => {
                let total_size = self.get_total_size()?;
                let real_offset = total_size as i64 + offset;
                if real_offset < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "negative real offset after seek",
                    ));
                }
                self.seek(io::SeekFrom::Start(real_offset as u64))
            }
            io::SeekFrom::Current(offset) => {
                let new_position = self.global_offset as i64 + offset;
                if new_position < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "negative real offset after seek",
                    ));
                }
                self.seek(io::SeekFrom::Start(new_position as u64))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Seek};

    use rstest::{fixture, rstest};

    use super::Multireader;

    type FakeReader = Multireader<Cursor<Vec<u8>>>;

    #[fixture]
    fn singleitem_reader() -> FakeReader {
        Multireader::new(vec![Cursor::new(vec![1, 2, 3])]).unwrap()
    }

    #[fixture]
    fn multiitem_reader() -> FakeReader {
        Multireader::new(vec![Cursor::new(vec![1, 2, 3]), Cursor::new(vec![4, 5])]).unwrap()
    }

    #[rstest]
    fn reader_should_read_from_one_item(mut singleitem_reader: FakeReader) {
        let mut buf = vec![];
        singleitem_reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, vec![1, 2, 3])
    }

    #[rstest]
    fn reader_should_seek_into_inner_item(mut singleitem_reader: FakeReader) {
        singleitem_reader.seek(std::io::SeekFrom::Start(1)).unwrap();
        assert_eq!(singleitem_reader.get_global_offset(), 1);
        assert_eq!(singleitem_reader.get_local_offset(), 1);

        let mut buf = vec![255];
        singleitem_reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf, vec![2]);
    }

    #[rstest]
    fn reader_should_seek_into_first_item(mut multiitem_reader: FakeReader) {
        multiitem_reader.seek(std::io::SeekFrom::Start(1)).unwrap();
        assert_eq!(multiitem_reader.get_global_offset(), 1);
        assert_eq!(multiitem_reader.get_local_offset(), 1);
        assert_eq!(multiitem_reader.get_current_item_index(), 0);

        let mut buf = vec![255];
        multiitem_reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf, vec![2]);
    }

    #[rstest]
    fn reader_should_seek_into_second_item(mut multiitem_reader: FakeReader) {
        multiitem_reader.seek(std::io::SeekFrom::Start(4)).unwrap();
        assert_eq!(multiitem_reader.get_global_offset(), 4);
        assert_eq!(multiitem_reader.get_local_offset(), 1);
        assert_eq!(multiitem_reader.get_current_item_index(), 1);

        let mut buf = vec![255];
        multiitem_reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf, vec![5]);
    }
}
