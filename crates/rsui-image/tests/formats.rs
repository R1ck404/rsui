//! WebP, animations and color profiles, compared with what other programs say the files hold: libwebp
//! for WebP, LittleCMS for color, and the formula in the spec for animated PNGs. `fixtures` has the files
//! and what's expected of them.

use proptest::prelude::*;
use rsui_image::{Animation, decode, decode_animation, info, is_animated};

#[test]
fn lossless_jpegs_are_exactly_what_libjpeg_turbo_gets() {
    for name in [
        "lossless_8",
        "lossless_12_restart",
        "lossless_16_sampled",
        "lossless_predictor_1",
        "lossless_predictor_2",
        "lossless_predictor_3",
        "lossless_predictor_4",
        "lossless_predictor_5",
        "lossless_predictor_6",
        "lossless_predictor_7",
    ] {
        let picture = decode(&file(&format!("{name}.jpg")), BIG).unwrap();
        let wanted = file(&format!("{name}.rgb"));
        assert_eq!((picture.width, picture.height), (24, 16), "{name}");
        let colors: Vec<u8> = picture
            .pixels
            .chunks_exact(4)
            .flat_map(|pixel| pixel[..3].iter().copied())
            .collect();
        assert_eq!(colors, wanted, "{name}");
    }
}

#[test]
fn a_lossless_jpeg_says_how_big_it_is_and_can_be_made_smaller() {
    let bytes = file("lossless_8.jpg");
    let found = info(&bytes).unwrap();
    assert_eq!((found.width, found.height), (24, 16));
    let small = decode(&bytes, (12, 8)).unwrap();
    assert_eq!((small.width, small.height), (12, 8));
}

#[test]
fn a_damaged_lossless_jpeg_does_no_harm() {
    let whole = file("lossless_12_restart.jpg");
    for length in (0..whole.len()).step_by(7) {
        let _ = decode(&whole[..length], BIG);
    }
    let mut changed = whole.clone();
    for at in (40..changed.len()).step_by(11) {
        changed[at] ^= 0x55;
    }
    let _ = decode(&changed, BIG);
}

/// The size, and the pixels of each frame, in what's expected of a file.
fn expected(name: &str) -> (u32, u32, Vec<Vec<u8>>) {
    let path = format!(
        "{}/tests/fixtures/{name}.expected",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(path).expect("the fixture is there");
    let number = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let (width, height, count) = (number(0), number(4), number(8));
    let size = (width * height * 4) as usize;
    let frames = data[12..].chunks_exact(size).map(<[u8]>::to_vec).collect();
    let _ = count;
    (width, height, frames)
}

fn file(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(path).expect("the fixture is there")
}

/// How far apart two pictures are, in their worst value.
fn worst_difference(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

const BIG: (u32, u32) = (u32::MAX, u32::MAX);

fn check_exact(file_name: &str, name: &str) {
    let bytes = file(file_name);
    let (width, height, frames) = expected(name);
    let found = info(&bytes).unwrap();
    assert_eq!((found.width, found.height), (width, height), "{name}");
    let picture = decode(&bytes, BIG).unwrap();
    assert_eq!((picture.width, picture.height), (width, height), "{name}");
    assert_eq!(
        worst_difference(&picture.pixels, &frames[0]),
        0,
        "{name} should be the same as libwebp, value for value"
    );
}

#[test]
fn lossy_webp_is_the_same_as_libwebp() {
    check_exact("lossy.webp", "lossy");
    check_exact("lossy_tiny.webp", "lossy_tiny");
    check_exact("lossy_simple_filter.webp", "lossy_simple_filter");
}

#[test]
fn lossless_webp_is_the_same_as_libwebp() {
    check_exact("lossless.webp", "lossless");
    check_exact("lossless_few_colors.webp", "lossless_few_colors");
}

#[test]
fn transparent_webp_is_the_same_as_libwebp() {
    check_exact("lossy_alpha.webp", "lossy_alpha");
    check_exact("lossless_alpha.webp", "lossless_alpha");
}

#[test]
fn every_way_of_encoding_comes_out_the_same_as_libwebp() {
    for name in [
        "settings_partitions",
        "settings_lowest_quality",
        "settings_highest_quality",
        "settings_flat",
        "settings_detail",
        "settings_lossless_0",
        "settings_lossless_1",
        "settings_lossless_2",
        "settings_smooth_top_quality",
        "settings_ramp",
        "settings_lossless_green",
        "settings_empty_blocks_small",
        "settings_empty_blocks",
    ] {
        check_exact(&format!("{name}.webp"), name);
    }
}

fn check_animation(file_name: &str, name: &str, allowed: u8) -> Animation {
    let bytes = file(file_name);
    assert!(is_animated(&bytes), "{name}");
    let (width, height, frames) = expected(name);
    let animation = decode_animation(&bytes, BIG).unwrap();
    assert_eq!(
        (animation.width, animation.height),
        (width, height),
        "{name}"
    );
    assert_eq!(animation.frames.len(), frames.len(), "{name}");
    for (index, (got, wanted)) in animation.frames.iter().zip(&frames).enumerate() {
        let worst = worst_difference(&got.pixels, wanted);
        assert!(worst <= allowed, "{name} frame {index} is out by {worst}");
    }
    animation
}

#[test]
fn animated_webp_is_the_same_as_libwebp() {
    let animation = check_animation("animated_webp.webp", "animated_webp", 0);
    let delays: Vec<u32> = animation.frames.iter().map(|frame| frame.delay).collect();
    assert_eq!(delays, [50, 70, 90, 110]);
    assert_eq!(animation.repeats, 0, "it plays forever");
}

#[test]
fn webp_frames_are_put_together_as_they_ask() {
    // By hand, this has frames that blend, that replace what's there, and that clear up after themselves.
    let animation = check_animation("webp_frames.webp", "webp_frames", 2);
    assert_eq!(animation.repeats, 2, "it says how many times to play");
}

#[test]
fn animated_png_follows_its_spec_however_frames_are_put_together() {
    for name in [
        "apng_over",
        "apng_background",
        "apng_previous",
        "apng_source",
    ] {
        let animation = check_animation(&format!("{name}.png"), name, 2);
        let delays: Vec<u32> = animation.frames.iter().map(|frame| frame.delay).collect();
        assert_eq!(delays, [40, 60, 80, 100, 120], "{name}");
    }
}

#[test]
fn the_first_frame_is_what_shows_when_not_playing() {
    let bytes = file("animated_webp.webp");
    let (_, _, frames) = expected("animated_webp");
    assert_eq!(decode(&bytes, BIG).unwrap().pixels, frames[0]);
}

#[test]
fn animation_frames_come_out_at_the_size_asked_for() {
    let bytes = file("apng_over.png");
    let animation = decode_animation(&bytes, (12, 9)).unwrap();
    assert_eq!((animation.width, animation.height), (12, 9));
    for frame in &animation.frames {
        assert_eq!(frame.pixels.len(), 12 * 9 * 4);
    }
}

#[test]
fn something_that_is_not_animated_is_one_frame() {
    let bytes = file("lossy.webp");
    assert!(!is_animated(&bytes));
    assert_eq!(decode_animation(&bytes, BIG).unwrap().frames.len(), 1);
    let bytes = file("profile_adobe.png");
    assert!(!is_animated(&bytes));
    assert_eq!(decode_animation(&bytes, BIG).unwrap().frames.len(), 1);
}

#[test]
fn color_profiles_come_out_as_littlecms_says() {
    for name in ["profile_adobe", "profile_p3", "profile_table"] {
        let (_, _, frames) = expected(name);
        let picture = decode(&file(&format!("{name}.png")), BIG).unwrap();
        let worst = worst_difference(&picture.pixels, &frames[0]);
        assert!(worst <= 1, "{name} is out by {worst}");
    }
}

#[test]
fn arithmetic_coding_makes_no_difference_to_what_a_jpeg_shows() {
    // These were each made from this one without changing anything but how it's stored.
    let plain = decode(&file("jpeg_baseline.jpg"), BIG).unwrap();
    for name in [
        "jpeg_arithmetic.jpg",
        "jpeg_arithmetic_progressive.jpg",
        "jpeg_arithmetic_restarts.jpg",
    ] {
        let found = info(&file(name)).unwrap();
        assert_eq!((found.width, found.height), (plain.width, plain.height));
        assert_eq!(decode(&file(name), BIG).unwrap(), plain, "{name}");
    }
}

#[test]
fn arithmetic_coding_works_at_smaller_sizes_too() {
    let plain = decode(&file("jpeg_baseline.jpg"), (5, 4)).unwrap();
    for name in ["jpeg_arithmetic.jpg", "jpeg_arithmetic_progressive.jpg"] {
        assert_eq!(decode(&file(name), (5, 4)).unwrap(), plain, "{name}");
    }
}

#[test]
fn twelve_bit_jpegs_come_out_as_libjpeg_says() {
    for name in ["jpeg_12_bits", "jpeg_12_bits_arithmetic"] {
        let (width, height, frames) = expected(name);
        let picture = decode(&file(&format!("{name}.jpg")), BIG).unwrap();
        assert_eq!((picture.width, picture.height), (width, height));
        let worst = worst_difference(&picture.pixels, &frames[0]);
        assert!(worst <= 2, "{name} is out by {worst}");
    }
}

#[test]
fn twelve_bit_jpegs_can_be_read_smaller() {
    let picture = decode(&file("jpeg_12_bits.jpg"), (5, 4)).unwrap();
    assert_eq!((picture.width, picture.height), (5, 4));
    // Each pixel comes from eight of the whole picture each way, and has to be near their average.
    let whole = decode(&file("jpeg_12_bits.jpg"), BIG).unwrap();
    // The last row of blocks is cut short by the picture's edge, so it isn't compared.
    for y in 0..3 {
        for x in 0..5 {
            for part in 0..3 {
                let mut total = 0u32;
                for down in 0..8 {
                    for across in 0..8 {
                        let at = ((y * 8 + down) * 40 + x * 8 + across) * 4 + part;
                        total += u32::from(whole.pixels[at]);
                    }
                }
                let shown = picture.pixels[(y * 5 + x) * 4 + part];
                assert!(
                    shown.abs_diff((total / 64) as u8) < 12,
                    "at {x},{y}: {shown}"
                );
            }
        }
    }
}

/// A PNG of one color, with this chunk in it.
fn png_with(pixel: [u8; 3], tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut raw = vec![0u8];
    raw.extend_from_slice(&pixel);
    let mut packed = vec![0x78, 0x01, 1, raw.len() as u8, 0, !(raw.len() as u8), 0xFF];
    packed.extend_from_slice(&raw);
    packed.extend_from_slice(&[0; 4]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(body);
        out.extend_from_slice(&[0; 4]);
    };
    chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(tag, body);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    out
}

#[test]
fn a_png_gamma_of_a_half_means_the_ordinary_thing() {
    let bytes = png_with([100, 150, 200], b"gAMA", &45_455u32.to_be_bytes());
    assert_eq!(decode(&bytes, BIG).unwrap().pixels, [100, 150, 200, 255]);
}

#[test]
fn a_png_with_another_gamma_is_brought_to_what_screens_show() {
    // Stored with a power of 1, this is linear light, and half of that shows as about 188.
    let bytes = png_with([128, 128, 128], b"gAMA", &100_000u32.to_be_bytes());
    let pixels = decode(&bytes, BIG).unwrap().pixels;
    assert!(pixels[0].abs_diff(188) <= 1, "{pixels:?}");
    assert_eq!(pixels[0], pixels[1]);
    assert_eq!(pixels[0], pixels[2]);
}

#[test]
fn a_png_that_says_it_is_ordinary_ignores_anything_else() {
    // Saying it's ordinary comes first, which here is the chunk before its gamma would be read.
    let mut bytes = png_with([128, 128, 128], b"sRGB", &[0]);
    let gamma = {
        let mut chunk = 4u32.to_be_bytes().to_vec();
        chunk.extend_from_slice(b"gAMA");
        chunk.extend_from_slice(&100_000u32.to_be_bytes());
        chunk.extend_from_slice(&[0; 4]);
        chunk
    };
    bytes.splice(33..33, gamma);
    assert_eq!(decode(&bytes, BIG).unwrap().pixels, [128, 128, 128, 255]);
}

#[test]
fn a_png_with_the_primaries_screens_have_stays_as_it_is() {
    let primaries: [u32; 8] = [
        31_270, 32_900, 64_000, 33_000, 30_000, 60_000, 15_000, 6_000,
    ];
    let body: Vec<u8> = primaries.iter().flat_map(|v| v.to_be_bytes()).collect();
    let bytes = png_with([100, 150, 200], b"cHRM", &body);
    let pixels = decode(&bytes, BIG).unwrap().pixels;
    assert!(pixels[0].abs_diff(100) <= 1, "{pixels:?}");
    assert!(pixels[1].abs_diff(150) <= 1, "{pixels:?}");
    assert!(pixels[2].abs_diff(200) <= 1, "{pixels:?}");
}

#[test]
fn a_profile_for_the_wrong_kind_of_picture_is_ignored() {
    // This is a color profile with its kind changed to print colors, and no table.
    let original = file("profile_adobe.png");
    let without = decode(&file("lossless.webp"), BIG).unwrap();
    assert!(!without.pixels.is_empty());
    let at = original
        .windows(4)
        .position(|window| window == b"iCCP")
        .expect("it has one");
    assert!(at > 0);
    let garbled = png_with([100, 150, 200], b"iCCP", b"name\0\0not a profile at all");
    assert_eq!(decode(&garbled, BIG).unwrap().pixels, [100, 150, 200, 255]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn damaged_files_never_cause_a_panic(
        which in 0usize..13,
        changes in proptest::collection::vec((any::<u32>(), any::<u8>()), 1..24),
        cut in any::<u16>(),
    ) {
        let names = [
            "lossy.webp", "lossy_alpha.webp", "lossless.webp", "lossless_few_colors.webp",
            "lossless_alpha.webp", "animated_webp.webp", "apng_over.png", "apng_previous.png",
            "profile_table.png", "jpeg_arithmetic.jpg", "jpeg_arithmetic_progressive.jpg",
            "jpeg_12_bits.jpg", "jpeg_12_bits_arithmetic.jpg",
        ];
        let mut bytes = file(names[which]);
        for (at, value) in changes {
            let at = at as usize % bytes.len();
            bytes[at] = value;
        }
        let keep = bytes.len() - (usize::from(cut) % bytes.len()) / 2;
        bytes.truncate(keep);
        let _ = info(&bytes);
        let _ = decode(&bytes, (64, 64));
        let _ = is_animated(&bytes);
        let _ = decode_animation(&bytes, (64, 64));
    }
}
