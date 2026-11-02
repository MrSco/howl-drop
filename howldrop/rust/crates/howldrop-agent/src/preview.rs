//! The preview the iPhone shows (FileIcon), where there's no Pillow: the
//! picture at most 540 pixels on the long side, as JPEG 2000 coded the way
//! an iPhone's own is and send.rs has Pillow code it (9/7 wavelet, colour
//! transform, one layer at 20:1). Read with the `image` crate, coded with
//! OpenJPEG.
//!
//! An iPhone (iOS 26) shows it on its "AirDrop Complete" banner, and its
//! generic icon for a plain JPEG. Its Accept prompt shows none, even for a
//! preview like an iPhone's own in size, coding and header: perhaps not
//! for a sender without an Apple ID record (docs/native-windows.md).

use std::ffi::c_void;
use std::path::Path;

use openjpeg_sys as opj;

/// The long side, at most.
const SIDE: u32 = 540;
/// One quality layer at this compression ratio, Pillow's on Linux. An
/// iPhone's own preview of a photo is nearer 4:1 (196 KB at 540 x 410);
/// that showed no better, and takes the air longer.
const RATE: f32 = 20.0;
/// Where the image header box's UnkC byte is: after the signature (12),
/// file type (20) and header box's own header (8), and 12 bytes into the
/// ihdr box past its header (8).
const UNKC: usize = 12 + 20 + 8 + 8 + 12;

/// The picture at `path` as a FileIcon; None for what can't be read here
/// (videos, documents, HEIC).
pub fn file_icon(path: &Path) -> Option<Vec<u8>> {
    let picture = image::open(path).ok()?.thumbnail(SIDE, SIDE).to_rgb8();
    jp2(picture.width(), picture.height(), picture.as_raw())
}

/// RGB, 8 bits a sample, as a JP2 file.
#[allow(unsafe_code)]
pub fn jp2(width: u32, height: u32, rgb: &[u8]) -> Option<Vec<u8>> {
    let pixels = width as usize * height as usize;
    if pixels == 0 || rgb.len() != pixels * 3 {
        return None;
    }
    let mut sink = Sink::default();
    // SAFETY: OpenJPEG's encoder as its documentation uses it. The image's
    // components are allocated by opj_image_create for width x height and
    // filled within that; the stream writes to `sink`, which outlives it;
    // everything made is destroyed before returning.
    unsafe {
        let mut parts = [opj::opj_image_cmptparm_t {
            dx: 1,
            dy: 1,
            w: width,
            h: height,
            x0: 0,
            y0: 0,
            prec: 8,
            bpp: 8,
            sgnd: 0,
        }; 3];
        let image = opj::opj_image_create(3, parts.as_mut_ptr(), opj::COLOR_SPACE::OPJ_CLRSPC_SRGB);
        if image.is_null() {
            return None;
        }
        (*image).x1 = width;
        (*image).y1 = height;
        let comps = std::slice::from_raw_parts((*image).comps, 3);
        for (c, comp) in comps.iter().enumerate() {
            if comp.data.is_null() {
                opj::opj_image_destroy(image);
                return None;
            }
            let data = std::slice::from_raw_parts_mut(comp.data, pixels);
            for (i, sample) in data.iter_mut().enumerate() {
                *sample = i32::from(rgb[i * 3 + c]);
            }
        }

        let mut params: opj::opj_cparameters_t = std::mem::zeroed();
        opj::opj_set_default_encoder_parameters(&mut params);
        params.tcp_numlayers = 1;
        params.tcp_rates[0] = RATE;
        params.cp_disto_alloc = 1;
        params.irreversible = 1;
        params.tcp_mct = 1;
        // Each resolution halves the picture: no more than it has pixels.
        while params.numresolution > 1 && 1u32 << (params.numresolution - 1) > width.min(height) {
            params.numresolution -= 1;
        }

        let codec = opj::opj_create_compress(opj::CODEC_FORMAT::OPJ_CODEC_JP2);
        let stream = opj::opj_stream_create(1 << 16, 0);
        let ok = !codec.is_null()
            && !stream.is_null()
            && {
                opj::opj_stream_set_user_data(stream, (&raw mut sink).cast(), None);
                opj::opj_stream_set_write_function(stream, Some(write));
                opj::opj_stream_set_skip_function(stream, Some(skip));
                opj::opj_stream_set_seek_function(stream, Some(seek));
                opj::opj_setup_encoder(codec, &mut params, image) != 0
            }
            && opj::opj_start_compress(codec, image, stream) != 0
            && opj::opj_encode(codec, stream) != 0
            && opj::opj_end_compress(codec, stream) != 0;
        if !stream.is_null() {
            opj::opj_stream_destroy(stream);
        }
        if !codec.is_null() {
            opj::opj_destroy_codec(codec);
        }
        opj::opj_image_destroy(image);
        if !ok || sink.data.len() <= UNKC || &sink.data[UNKC - 16..UNKC - 12] != b"ihdr" {
            return None;
        }
        // UnkC 1, as an iPhone's own says: the colour space may be unknown
        // (OpenJPEG writes 0).
        sink.data[UNKC] = 1;
        Some(sink.data)
    }
}

/// Where OpenJPEG writes, seeking back to fill in box lengths.
#[derive(Default)]
struct Sink {
    data: Vec<u8>,
    at: usize,
}

impl Sink {
    fn reach(&mut self, at: usize) {
        if self.data.len() < at {
            self.data.resize(at, 0);
        }
    }
}

#[allow(unsafe_code)]
unsafe extern "C" fn write(buffer: *mut c_void, n: usize, sink: *mut c_void) -> usize {
    // SAFETY: OpenJPEG passes the Sink given as user data, and n bytes.
    let (sink, bytes) = unsafe {
        (
            &mut *sink.cast::<Sink>(),
            std::slice::from_raw_parts(buffer.cast::<u8>(), n),
        )
    };
    let end = sink.at + n;
    sink.reach(end);
    sink.data[sink.at..end].copy_from_slice(bytes);
    sink.at = end;
    n
}

#[allow(unsafe_code)]
unsafe extern "C" fn skip(n: i64, sink: *mut c_void) -> i64 {
    // SAFETY: as in write.
    let sink = unsafe { &mut *sink.cast::<Sink>() };
    match usize::try_from(sink.at as i64 + n) {
        Ok(at) => {
            sink.at = at;
            sink.reach(at);
            n
        }
        Err(_) => -1,
    }
}

#[allow(unsafe_code)]
unsafe extern "C" fn seek(to: i64, sink: *mut c_void) -> opj::OPJ_BOOL {
    // SAFETY: as in write.
    let sink = unsafe { &mut *sink.cast::<Sink>() };
    match usize::try_from(to) {
        Ok(at) => {
            sink.at = at;
            sink.reach(at);
            1
        }
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_come_out_as_jp2() {
        // Noise, which no coder can take below the rate asked for.
        let (w, h) = (300u32, 200u32);
        let mut x = 0x2545_f491_u32;
        let rgb: Vec<u8> = (0..w * h * 3)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let out = jp2(w, h, &rgb).unwrap();
        // The JP2 signature box, then the file type box naming jp2.
        assert_eq!(
            out[..12],
            [0, 0, 0, 12, b'j', b'P', b' ', b' ', 13, 10, 0x87, 10]
        );
        assert_eq!(&out[20..24], b"jp2 ");
        assert_eq!(&out[UNKC - 16..UNKC - 12], b"ihdr");
        assert_eq!(out[UNKC], 1);
        // About 20:1 of 180 KB.
        assert!(
            out.len() > 4_000 && out.len() < 12_000,
            "{} bytes",
            out.len()
        );
        // A tiny one, with fewer resolutions.
        assert!(jp2(8, 5, &[200; 8 * 5 * 3]).is_some());
        assert!(jp2(0, 5, &[]).is_none());
    }

    /// The preview of PREVIEW_IN, written to PREVIEW_OUT, to look at:
    /// `cargo test -p howldrop-agent preview_of_a_file -- --ignored`.
    #[test]
    #[ignore]
    fn preview_of_a_file() {
        let input = std::env::var("PREVIEW_IN").unwrap();
        let out = file_icon(Path::new(&input)).unwrap();
        std::fs::write(std::env::var("PREVIEW_OUT").unwrap(), out).unwrap();
    }
}
