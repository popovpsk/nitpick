use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub const MAX_LINE: usize = 32 * 1024 * 1024;

pub struct Lines<R> {
    reader: BufReader<R>,
    pending: Vec<u8>,
    skipping: bool,
    limit: usize,
}

impl<R: AsyncRead + Unpin> Lines<R> {
    pub fn new(reader: R) -> Self {
        Self::with_limit(reader, MAX_LINE)
    }

    pub fn with_limit(reader: R, limit: usize) -> Self {
        Self {
            reader: BufReader::new(reader),
            pending: Vec::new(),
            skipping: false,
            limit,
        }
    }

    pub async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            let buf = self.reader.fill_buf().await?;
            if buf.is_empty() {
                if std::mem::take(&mut self.skipping) {
                    log::warn!(
                        "dropping a line longer than {} bytes at end of stream",
                        self.limit
                    );
                    return Ok(None);
                }
                if self.pending.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(self.take_line()));
            }
            let newline = buf.iter().position(|&byte| byte == b'\n');
            let chunk = newline.unwrap_or(buf.len());
            if !self.skipping {
                if self.pending.len() + chunk > self.limit {
                    self.skipping = true;
                    self.pending = Vec::new();
                } else {
                    self.pending.extend_from_slice(&buf[..chunk]);
                }
            }
            self.reader.consume(chunk + usize::from(newline.is_some()));
            if newline.is_some() {
                if std::mem::take(&mut self.skipping) {
                    log::warn!("dropping a line longer than {} bytes", self.limit);
                    continue;
                }
                return Ok(Some(self.take_line()));
            }
        }
    }

    fn take_line(&mut self) -> String {
        let mut bytes = std::mem::take(&mut self.pending);
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        match String::from_utf8(bytes) {
            Ok(line) => line,
            Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
        }
    }
}
