use tokio::io::{AsyncRead, BufReader};

/// The threshold determines the minimum size of data that should be compressed.
/// Data smaller than the threshold will not be compressed.
/// Data smaller than the threshold will not be compressed.
pub type CompressionThreshold = usize;

/// The level controls the amount of compression applied to the data.
/// Higher levels generally result in higher compression ratios, but also
/// increase CPU usage.
pub type CompressionLevel = u32;
pub enum DecompressionReader<R: AsyncRead + Unpin> {
    Decompress(async_compression::tokio::bufread::ZlibDecoder<BufReader<R>>),
    None(R),
}

impl<R: AsyncRead + Unpin> AsyncRead for DecompressionReader<R> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Decompress(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
            Self::None(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
        }
    }
}
