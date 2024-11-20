use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{BufWriter, Write},
    iter::once,
    num::NonZeroU32,
    ops::Rem,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use bpaf::*;
use image::{codecs::gif::GifEncoder, RgbaImage};
use kdam::{par_tqdm, tqdm};
use palette::{IntoColor, LinSrgba, Okhsva, OklabHue, Srgba};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use webp_animation::{Encoder, EncoderOptions};

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options)]
struct Options {
    #[bpaf(long, fallback(9))]
    order: u8,
    #[bpaf(short, long, fallback("oklab_hue".to_owned()))]
    function: String,
    #[bpaf(short, long, fallback(256))]
    frames: usize,
    #[bpaf(short('r'), long, fallback(30))]
    framerate: u32,
    #[bpaf(short, long, fallback(NonZeroU32::new(1).unwrap()))]
    loops: NonZeroU32,
    #[bpaf(short, long)]
    bitrate: Option<String>,
    #[bpaf(positional, fallback("out.webp".into()))]
    filename: PathBuf,
}

#[derive(Debug, Clone)]
struct Params {
    order: u8,
    image_size: u32,
    num_pixels: u64,
    frames: usize,
    framerate: u32,
    loops: NonZeroU32,
    bitrate: Option<String>,
    filename: PathBuf,
}

impl Params {
    fn new(options: Options) -> Self {
        let order = options.order;
        let image_size = 2u32.pow(order as u32);
        let num_pixels = (image_size as u64).pow(2);

        Self {
            order,
            image_size,
            num_pixels,
            frames: options.frames,
            framerate: options.framerate,
            loops: options.loops,
            bitrate: options.bitrate,
            filename: options.filename,
        }
    }

    fn gen_image<F>(&self, color: F, offset: u64) -> RgbaImage
    where
        F: Fn(u64, u64) -> Srgba<u8>,
    {
        RgbaImage::from_fn(self.image_size, self.image_size, |x, y| {
            let i = (fast_hilbert::xy2h(x, y, self.order) + offset) % self.num_pixels;
            let (r, g, b, a) = color(i, self.num_pixels).into_components();
            // dbg!((r, g, b));
            image::Rgba([r, g, b, a])
        })
    }

    fn write_gif<I>(&self, frames: I) -> Result<()>
    where
        I: ParallelIterator<Item = RgbaImage> + IndexedParallelIterator,
    {
        let mut frames_vec = Vec::with_capacity(self.frames);
        par_tqdm!(frames.map_with(self.framerate, |framerate, frame| {
            image::Frame::from_parts(
                frame,
                0,
                0,
                image::Delay::from_numer_denom_ms(1000, *framerate),
            )
        }))
        .collect_into_vec(&mut frames_vec);

        let file = BufWriter::new(File::create(&self.filename).context("Failed to open file")?);
        let mut encoder = GifEncoder::new(file);
        encoder
            .encode_frames(tqdm!(frames_vec.into_iter()))
            .context("failed to write frames")?;

        Ok(())
    }

    fn write_webp<I>(&self, frames: I) -> Result<()>
    where
        I: ParallelIterator<Item = RgbaImage> + IndexedParallelIterator,
    {
        let mut frames_vec = Vec::with_capacity(self.frames);
        par_tqdm!(frames).collect_into_vec(&mut frames_vec);

        let mut webp_encoder = Encoder::new_with_options(
            (self.image_size, self.image_size),
            EncoderOptions {
                minimize_size: true,
                ..Default::default()
            },
        )
        .context("Failed to initialize webp encoder")?;

        let mut timestamp: f64 = 0.0;
        for frame in tqdm!(frames_vec.into_iter()) {
            webp_encoder
                .add_frame(frame.as_flat_samples().samples, timestamp.round() as i32)
                .context("Failed to add frame to webp")?;
            timestamp += 1000.0 / self.framerate as f64;
        }

        let webp_data = webp_encoder
            .finalize(timestamp.round() as i32)
            .context("Failed to finalize webp")?;
        let mut file = BufWriter::new(File::create(&self.filename).context("Failed to open file")?);
        file.write_all(webp_data.as_ref())
            .context("Failed to write webp to file")?;

        Ok(())
    }

    fn write_frames<I>(&self, frames: I, out_dir: Option<&Path>) -> Result<()>
    where
        I: ParallelIterator<Item = RgbaImage> + IndexedParallelIterator,
    {
        let out_dir = out_dir.unwrap_or(&self.filename);

        if self.filename.is_dir() {
            fs::remove_dir_all(out_dir).context("Failed to remove existing output dir")?;
        }
        fs::create_dir_all(out_dir).context("Failed to create output dir")?;

        par_tqdm!(frames.enumerate()).try_for_each_with(out_dir, |out_dir, (i, frame)| {
            frame
                .save(out_dir.join(format!("frame_{i:05}.png")))
                .with_context(|| format!("Failed to save frame {i}"))
        })?;

        Ok(())
    }

    fn frames_to_webm(&self, frames_dir: &Path) -> Result<()> {
        std::process::Command::new("ffmpeg")
            .args(
                [
                    "-y",
                    "-framerate",
                    &self.framerate.to_string(),
                    "-stream_loop",
                    &(self.loops.get() - 1).to_string(),
                    "-pattern_type",
                    "glob",
                    "-i",
                ]
                .into_iter()
                .map(OsStr::new)
                .chain(once(frames_dir.join("*.png").as_os_str()))
                .chain(
                    [
                        "-c:v",
                        "libvpx-vp9",
                        // "-deadline",
                        // "best",
                        // "-cpu-used",
                        // "1"
                    ]
                    .map(OsStr::new),
                )
                .chain(
                    self.bitrate
                        .as_ref()
                        .map(|b| [OsStr::new("-b:v"), OsStr::new(b)].into_iter())
                        .into_iter()
                        .flatten(),
                )
                .chain(once(self.filename.as_os_str())),
            )
            .spawn()
            .context("Failed to run FFMpeg")?
            .wait()
            .context("FFMpeg failed")?;

        Ok(())
    }
}

fn oklab_hue(i: u64, size: u64) -> Srgba<u8> {
    let degrees = i as f64 / size as f64 * 360.0;
    let color = Okhsva::new(OklabHue::new(degrees), 1.0, 1.0, 1.0);
    let rgb_color: LinSrgba<f64> = color.into_color();
    rgb_color.into_encoding()
}

fn oklab_hue_sine_value(i: u64, size: u64) -> Srgba<u8> {
    let progress = i as f64 / size as f64;
    let hue = OklabHue::new(progress * 360.0);
    let sine_cycles = 8.0;
    let value = (progress * 2.0 * std::f64::consts::PI * sine_cycles).sin() * 0.375 + 0.625;
    let color = Okhsva::new(hue, 1.0, value, 1.0);
    let rgb_color: LinSrgba<f64> = color.into_color();
    rgb_color.into_encoding()
}

fn square_value(i: u64, size: u64) -> Srgba<u8> {
    let progress = (i as f64 / size as f64 * 2.0).rem(1.0);
    let value = -(progress * 2.0 - 1.0).powf(2.0) + 1.0;
    let color = Okhsva::new(OklabHue::new(0.0), 0.0, value, 1.0);
    let rgb_color: LinSrgba<f64> = color.into_color();
    rgb_color.into_encoding()
}

fn square_channel(progress: f64) -> f64 {
    (-(progress * 4.0 - 2.0).powf(2.0) + 1.0).max(0.0)
}

fn square_linsrgb_channels(i: u64, size: u64) -> Srgba<u8> {
    let progress = i as f64 / size as f64;
    let red_progress = (progress + (1.0 / 3.0)).rem_euclid(1.0);
    let green_progress = progress;
    let blue_progress = (progress - (1.0 / 3.0)).rem_euclid(1.0);
    let color = LinSrgba::new(
        square_channel(red_progress),
        square_channel(green_progress),
        square_channel(blue_progress),
        1.0,
    );
    color.into_encoding()
}

fn main() {
    let opts = options().run();
    let function = match &*opts.function {
        "oklab_hue" => oklab_hue,
        "oklab_hue_sine_value" => oklab_hue_sine_value,
        "square_value" => square_value,
        "square_linsrgb_channels" => square_linsrgb_channels,
        _ => panic!("unknown function {}", opts.function),
    };
    let params = Params::new(opts);

    let frames = (0..params.frames)
        .into_par_iter()
        .map_with(params.clone(), |params, i| {
            let offset = i as u64 * params.num_pixels / params.frames as u64;
            params.gen_image(function, offset)
        });

    match params.filename.extension().and_then(|ext| ext.to_str()) {
        Some("gif") => params
            .write_gif(frames)
            .context("Failed to write gif")
            .unwrap(),
        Some("webp") => params
            .write_webp(frames)
            .context("Failed to write webp")
            .unwrap(),
        Some("webm") => {
            let temp_frames_path = Path::new("_frames_out");
            params
                .write_frames(frames, Some(temp_frames_path))
                .context("Failed to write frames")
                .unwrap();
            params
                .frames_to_webm(temp_frames_path)
                .context("Failed to convert frames to webm")
                .unwrap();
        }
        None => params
            .write_frames(frames, None)
            .context("Failed to write frames")
            .unwrap(),
        Some(ext) => panic!("unknown format '{}'", ext),
    }
}
