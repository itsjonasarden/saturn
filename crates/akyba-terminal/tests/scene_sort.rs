//! Keep the renderer's optimized glyph sort identical to stable paint ordering.
use gpui_kit::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, ContentMask, Hsla, MonochromeSprite, Scene, TileId,
    TransformationMatrix,
};

fn sprite(index: u32, order: u32, tile: u32) -> MonochromeSprite {
    MonochromeSprite {
        order,
        pad: index,
        bounds: Bounds::default(),
        content_mask: ContentMask::default(),
        color: Hsla::default(),
        tile: AtlasTile {
            texture_id: AtlasTextureId { index: 0, kind: AtlasTextureKind::Monochrome },
            tile_id: TileId(tile),
            padding: 0,
            bounds: Bounds::default(),
        },
        transformation: TransformationMatrix::default(),
    }
}

#[test]
fn glyph_sort_preserves_stable_paint_order() {
    for count in [0, 1, 16, 128, 8192] {
        for distribution in 0..4 {
            let input: Vec<_> = (0..count)
                .map(|index| {
                    let order = match distribution {
                        0 => index / 80,
                        1 => index % 37,
                        2 => 0,
                        _ => index,
                    };
                    sprite(index, order, (index * 13) % 97)
                })
                .collect();
            let mut expected = input.clone();
            expected.sort_by_key(|glyph| (glyph.order, glyph.tile.tile_id));
            let mut scene = Scene::default();
            scene.monochrome_sprites = input;
            scene.finish();
            let indices = |glyphs: &[MonochromeSprite]| glyphs.iter().map(|glyph| glyph.pad).collect::<Vec<_>>();
            assert_eq!(indices(&scene.monochrome_sprites), indices(&expected));
            scene.finish();
            assert_eq!(indices(&scene.monochrome_sprites), indices(&expected));
        }
    }
}

/// Run manually in release mode; timing is deliberately not a CI assertion.
#[test]
#[ignore = "manual renderer benchmark; run with --release --ignored --nocapture"]
fn glyph_sort_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    println!("glyph instance bytes: {}", std::mem::size_of::<MonochromeSprite>());
    for count in [128_u32, 1024, 8192, 32768] {
        for distribution in 0..4 {
            let input: Vec<_> = (0..count)
                .map(|index| {
                    let order = match distribution {
                        0 => index / 80,
                        1 => index % 37,
                        2 => 0,
                        _ => index,
                    };
                    sprite(index, order, (index * 13) % 97)
                })
                .collect();
            let mut timings = [Vec::new(), Vec::new()];
            for round in 0..7 {
                for method in [round % 2, (round + 1) % 2] {
                    let mut scene = Scene::default();
                    scene.monochrome_sprites = input.clone();
                    let repeats = 1_000_000 / count;
                    let start = Instant::now();
                    for _ in 0..repeats {
                        scene.monochrome_sprites.copy_from_slice(&input);
                        if method == 0 {
                            scene.monochrome_sprites.sort_by_key(|glyph| (glyph.order, glyph.tile.tile_id));
                        } else {
                            scene.finish();
                        }
                        black_box(&scene.monochrome_sprites);
                    }
                    timings[method].push(start.elapsed().as_secs_f64() * 1e6 / f64::from(repeats));
                }
            }
            for times in &mut timings {
                times.sort_by(f64::total_cmp);
            }
            println!(
                "glyphs={count} distribution={distribution} original_us={:.2} optimized_us={:.2} ratio={:.3}",
                timings[0][3],
                timings[1][3],
                timings[1][3] / timings[0][3]
            );
        }
    }
}
