// Captures promo.html frame by frame, and encodes the frames with the song's audio into one MP4.
//
//   node capture.cjs <stage/promo.html> <song.wav> <out.mp4> [fps]
//
// The page reads its frames from `frames/` beside it, and this reads the bar times from
// `frames/bars.json` and hands them to the page. The `promo` example in km-display writes both.
//
// `tools/dev/promo-video.sh` stages the page and runs this. It needs Playwright, which it finds
// through NODE_PATH, and ffmpeg with libx264. KM_CHROME names a browser to use instead of
// Playwright's own.
//
// Each frame is a screenshot of the page after `seek(t)` resolves. ffmpeg reads the screenshots from
// a pipe, so no frame of the finished video touches the disk as an image.

"use strict";

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { chromium } = require("playwright");

async function main() {
  const [page_path, wav, out, fps_arg] = process.argv.slice(2);
  if (!page_path || !wav || !out) {
    console.error("usage: node capture.cjs <stage/promo.html> <song.wav> <out.mp4> [fps]");
    process.exit(2);
  }
  const fps = Number(fps_arg || 30);

  const browser = await chromium.launch({
    executablePath: process.env.KM_CHROME || undefined,
    args: ["--hide-scrollbars", "--force-color-profile=srgb"],
  });
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  const url = pathToFileURL(path.resolve(page_path));
  url.search = `?fps=${fps}`;
  await page.goto(url.href, { waitUntil: "load" });
  await page.evaluate(() => document.fonts.ready);
  const bars = JSON.parse(fs.readFileSync(path.join(path.dirname(page_path), "frames", "bars.json"), "utf8"));
  await page.evaluate((seconds) => window.setBars(seconds), bars);
  const duration = await page.evaluate(() => window.DURATION);
  const frames = Math.round(duration * fps);

  // ffmpeg's loudnorm levels the music toward -16 LUFS, the level a video site plays at. The music
  // fades out over the last second and a half, and stops with the last frame.
  const fade = Math.max(0, duration - 1.5).toFixed(3);
  const ffmpeg = spawn(
    "ffmpeg",
    [
      "-hide_banner", "-loglevel", "error", "-y",
      "-f", "image2pipe", "-framerate", String(fps), "-c:v", "png", "-i", "-",
      "-i", wav,
      "-filter_complex", `[1:a]atrim=0:${duration},loudnorm=I=-16:TP=-1.5:LRA=11,aresample=48000,afade=t=out:st=${fade}:d=1.5[a]`,
      "-map", "0:v", "-map", "[a]",
      "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p",
      "-c:a", "aac", "-b:a", "192k",
      "-movflags", "+faststart",
      out,
    ],
    { stdio: ["pipe", "inherit", "inherit"] },
  );
  const finished = new Promise((resolve, reject) => {
    ffmpeg.on("error", reject);
    ffmpeg.on("close", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg exited with ${code}`))));
  });

  for (let n = 0; n < frames; n++) {
    await page.evaluate((t) => window.seek(t), n / fps);
    const png = await page.screenshot({ type: "png" });
    if (!ffmpeg.stdin.write(png)) {
      await new Promise((resolve) => ffmpeg.stdin.once("drain", resolve));
    }
    if (n % fps === 0) process.stdout.write(`\r  frame ${n} of ${frames}`);
  }
  process.stdout.write(`\r  frame ${frames} of ${frames}\n`);
  ffmpeg.stdin.end();
  await browser.close();
  await finished;
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
