// Needed for simple implementation
#![feature(portable_simd)]

use std::sync::Arc;

use async_trait::async_trait;
use breakwater_core::framebuffer::FrameBuffer;
use snafu::Snafu;
use tokio::io::AsyncWriteExt;

pub mod implementations;

#[derive(Debug, Snafu)]
pub enum ParserError {
    #[snafu(display("Failed to write to TCP socket"))]
    WriteToTcpSocket { source: std::io::Error },
}

#[async_trait]
pub trait Parser {
    async fn parse(
        &mut self,
        buffer: &[u8],
        fb: &Arc<FrameBuffer>,
        stream: impl AsyncWriteExt + Send + Unpin,
    ) -> Result<usize, ParserError>;

    // Sadly this cant be const (yet?) (https://github.com/rust-lang/rust/issues/71971 and https://github.com/rust-lang/rfcs/pull/2632)
    fn parser_lookahead() -> usize;
}
