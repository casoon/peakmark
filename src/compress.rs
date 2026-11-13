//! The size of a module on the wire.

use std::io::Write;

/// The settings of both encoders, as recorded in the baseline. A baseline taken
/// with other settings is not comparable for gzip and brotli.
pub(crate) const SETTINGS: &str = "gzip-9 brotli-11-w22";

/// Length in bytes of `bytes` compressed with gzip at level 9.
#[must_use]
pub(crate) fn gzip_len(bytes: &[u8]) -> u64 {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).expect("writing to a Vec");
    encoder.finish().expect("writing to a Vec").len() as u64
}

/// Length in bytes of `bytes` compressed with brotli at quality 11 and a
/// 22-bit window, the settings of the reference encoder and of `node:zlib`.
/// This encoder is a Rust port: on the same settings it lands about 0.1 %
/// above the reference, not on the same byte.
#[must_use]
pub(crate) fn brotli_len(bytes: &[u8]) -> u64 {
    let mut compressed = Vec::new();
    brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22)
        .write_all(bytes)
        .expect("writing to a Vec");
    compressed.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repetitive_input_compresses_well_below_its_length() {
        let bytes = b"peakmark-".repeat(500);
        assert!(gzip_len(&bytes) < 100);
        assert!(brotli_len(&bytes) < 100);
    }
}
