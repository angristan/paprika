use std::io::{Cursor, Seek, SeekFrom, Write};

pub(super) struct BoundedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: usize,
    pub(super) limit_exceeded: bool,
}

impl BoundedBuffer {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
            limit_exceeded: false,
        }
    }

    pub(super) fn into_inner(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let end = self.inner.position().saturating_add(buffer.len() as u64);
        if self.limit_exceeded || end > self.limit as u64 {
            // Continue as a sink after crossing the limit. This lets container
            // writers finish cleanly without allocating more memory; callers
            // inspect `limit_exceeded` and discard the truncated result.
            self.limit_exceeded = true;
            return Ok(buffer.len());
        }
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for BoundedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let next = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => add_signed_offset(self.inner.get_ref().len() as u64, offset),
            SeekFrom::Current(offset) => add_signed_offset(self.inner.position(), offset),
        };
        let Some(next) = next else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek before start of image buffer",
            ));
        };
        if next > self.limit as u64 {
            self.limit_exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "encoded image exceeds its memory budget",
            ));
        }
        self.inner.seek(SeekFrom::Start(next))
    }
}

fn add_signed_offset(base: u64, offset: i64) -> Option<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    }
}
