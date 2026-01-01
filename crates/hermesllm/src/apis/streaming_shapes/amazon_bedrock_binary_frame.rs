use aws_smithy_eventstream::frame::DecodedFrame;
use aws_smithy_eventstream::frame::MessageFrameDecoder;
use bytes::Buf;

/// AWS Event Stream frame decoder wrapper
pub struct BedrockBinaryFrameDecoder<B>
where
    B: Buf,
{
    decoder: MessageFrameDecoder,
    buffer: B,
}

impl BedrockBinaryFrameDecoder<bytes::BytesMut> {
    /// This is a convenience constructor that creates a BytesMut buffer internally
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let buffer = bytes::BytesMut::from(bytes);
        Self {
            decoder: MessageFrameDecoder::new(),
            buffer,
        }
    }
}

impl<B> BedrockBinaryFrameDecoder<B>
where
    B: Buf,
{
    pub fn new(buffer: B) -> Self {
        Self {
            decoder: MessageFrameDecoder::new(),
            buffer,
        }
    }

    pub fn decode_frame(&mut self) -> Option<DecodedFrame> {
        self.decoder.decode_frame(&mut self.buffer).ok()
    }

    pub fn buffer_mut(&mut self) -> &mut B {
        &mut self.buffer
    }

    /// Check if there are any bytes remaining in the buffer
    pub fn has_remaining(&self) -> bool {
        self.buffer.has_remaining()
    }
}
