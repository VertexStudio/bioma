use askama::Template;
use clap::Parser;
use std::io::prelude::*;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tracing::{error, info};

#[derive(Template)]
#[template(path = "ffmpeg.txt")]
struct FfmpegTemplate<'a> {
    input_source: &'a str,
}

#[derive(Parser, Debug)]
#[command(version = "1.0", about = "a CLI tool for running ffmpeg with AI agents", long_about = None)]
struct Cli {
    /// ffmpeg input source
    #[arg(short, long, help = "input source for ffmpeg")]
    source: String,
}

#[tokio::main]
async fn main() {
    let filter = match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => tracing_subscriber::EnvFilter::new("info"),
    };

    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    color_backtrace::install();
    color_backtrace::BacktracePrinter::new().message("BOOM! 💥").install(color_backtrace::default_output_stream());

    // Parse the CLI arguments
    let args = Cli::parse();
    let source = PathBuf::from(args.source);
    info!("Source: {:?}", source);

    // Render the template
    let template = FfmpegTemplate { input_source: &source.to_str().unwrap() };
    let rendered = template.render().unwrap();
    info!("{}", rendered);

    // Parse command line with shlex
    let cmdline = shlex::split(&rendered).unwrap();
    info!("{:?}", &cmdline);

    let cmd = &cmdline[0];
    let cmd_args = &cmdline[1..];

    let mut child = Command::new(cmd)
        .args(cmd_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to execute ffmpeg command");

    let mut stdout = child.stdout.take().expect("Failed to take stdout");

    let width: u32 = 1920;
    let height: u32 = 1080;
    let frame_size = (width * height * 3) as usize; // 3 bytes per pixel for RGB8
    let mut frame_index = 0;

    let mut buffer = Vec::with_capacity(frame_size);

    // Loop for reading frames from stdout
    loop {
        // Non-blocking read from the BufReader
        let mut temp_buffer = [0u8; 1024]; // Adjust the buffer size as needed
        match stdout.read(&mut temp_buffer) {
            Ok(0) => {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if status.success() {
                            info!("[{:?}] ffmpeg exited successfully", &source);
                        } else {
                            error!("[{:?}] ffmpeg exited with error: {}", &source, status);
                        }
                        break;
                    }
                    Ok(None) => (),
                    Err(e) => {
                        error!("[{:?}] Error attempting to wait: {}", &source, e);
                        break;
                    }
                }
                // No data available, continue to the next iteration of the loop
                continue;
            }
            Ok(size) => {
                // Append the read data to the frame buffer
                buffer.extend_from_slice(&temp_buffer[..size]);

                // Check if we have a complete frame
                if buffer.len() >= frame_size {
                    // Save the frame to a file
                    let image_data: Vec<u8> = buffer[..frame_size].to_vec();
                    let frame_name = format!("frame-{:04}", frame_index);
                    let frame_path = format!("output/{}.jpg", frame_name);

                    // Convert the image to a JPEG-encoded byte array
                    let mut jpeg_bytes: Vec<u8> = Vec::new();
                    let jpeg_cursor = Cursor::new(&mut jpeg_bytes);
                    let mut jpeg_enc = image::codecs::jpeg::JpegEncoder::new_with_quality(jpeg_cursor, 85);
                    jpeg_enc.encode(&image_data, width, height, image::ExtendedColorType::Rgb8).unwrap();

                    // Save a copy to file
                    let image = image::load_from_memory_with_format(&jpeg_bytes, image::ImageFormat::Jpeg).unwrap();
                    image.save(frame_path).unwrap();
                    info!("Saved frame: {}", frame_name);

                    // Remove the saved frame from the buffer
                    buffer.drain(..frame_size);

                    frame_index += 1;
                }
            }
            Err(e) => {
                error!("Error reading ffmpeg stdout: {}", e);
                break;
            }
        }
    }
}
