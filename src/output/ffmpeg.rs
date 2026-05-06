//! Pipe raw RGBA frames into FFmpeg over stdin, mux with the original
//! audio track, encode to whatever container the output extension implies.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{anyhow, Context, Result};

pub struct VideoEncoder {
    child: Child,
}

impl VideoEncoder {
    pub fn start(out: &Path, width: u32, height: u32, fps: u32, audio: &Path) -> Result<Self> {
        let child = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                // Video stream from stdin: raw RGBA pixels.
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
                &format!("{width}x{height}"),
                "-r",
                &fps.to_string(),
                "-i",
                "-",
                // Audio from the original file.
                "-i",
                audio.to_str().context("non-utf8 audio path")?,
                // Encoder settings: H.264, yuv420p for compatibility, AAC audio.
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "18",
                "-preset",
                "medium",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                // Stop when the shorter input ends (the rendered video).
                "-shortest",
                out.to_str().context("non-utf8 output path")?,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawning ffmpeg (is it installed and on PATH?)")?;

        Ok(Self { child })
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<()> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("ffmpeg stdin closed"))?;
        stdin.write_all(rgba).context("writing frame to ffmpeg")?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        // Closing stdin signals end-of-stream.
        drop(self.child.stdin.take());
        let status = self.child.wait().context("waiting for ffmpeg")?;
        if !status.success() {
            return Err(anyhow!("ffmpeg exited with status {status}"));
        }
        Ok(())
    }
}
