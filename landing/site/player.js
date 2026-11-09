// The demo player: poster first, then the video with the designed controls. Until the demo file is
// published (/media/demo.mp4), the play button says so instead of opening an empty player.
(() => {
  const player = document.querySelector(".player");
  if (!player) return;
  const video = player.querySelector("video");
  const play = player.querySelector(".play");
  const watch = player.querySelector(".watch");
  const now = player.querySelector(".now");
  const durations = player.querySelectorAll(".duration");
  const progress = player.querySelector(".progress");
  const played = player.querySelector(".played");
  const cc = player.querySelector(".cc");
  const mute = player.querySelector(".mute");
  const full = player.querySelector(".full");
  const src = player.dataset.src;

  const clock = (s) => {
    s = Math.max(0, Math.floor(s || 0));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  const state = (s) => { player.dataset.state = s; };

  let available = null;
  const check = () => {
    if (available !== null) return Promise.resolve(available);
    return fetch(src, { method: "HEAD" })
      .then((r) => (available = r.ok && (r.headers.get("content-type") || "").startsWith("video/")))
      .catch(() => (available = false));
  };

  const start = () =>
    check().then((ok) => {
      if (!ok) {
        state("soon");
        watch.textContent = "DEMO LANDS SOON";
        return;
      }
      if (!video.currentSrc) {
        video.src = src;
        video.preload = "auto";
      }
      state("playing");
      return video.play().catch(() => state("paused"));
    });

  play.addEventListener("click", start);
  video.addEventListener("click", () => (video.paused ? video.play() : video.pause()));
  video.addEventListener("play", () => state("playing"));
  video.addEventListener("pause", () => state("paused"));
  video.addEventListener("ended", () => state("paused"));
  video.addEventListener("loadedmetadata", () => {
    durations.forEach((d) => (d.textContent = clock(video.duration)));
  });
  video.addEventListener("timeupdate", () => {
    const p = video.duration ? (video.currentTime / video.duration) * 100 : 0;
    now.textContent = clock(video.currentTime);
    played.style.setProperty("--p", `${p}%`);
    progress.setAttribute("aria-valuenow", String(Math.round(p)));
  });

  const seek = (fraction) => {
    if (!video.duration) return;
    video.currentTime = Math.min(1, Math.max(0, fraction)) * video.duration;
  };
  progress.addEventListener("click", (e) => {
    const r = progress.getBoundingClientRect();
    seek((e.clientX - r.left) / r.width);
  });
  progress.addEventListener("keydown", (e) => {
    if (!video.duration) return;
    const step = e.key === "ArrowRight" ? 5 : e.key === "ArrowLeft" ? -5 : 0;
    if (step) {
      e.preventDefault();
      video.currentTime = Math.min(video.duration, Math.max(0, video.currentTime + step));
    }
  });

  cc.addEventListener("click", () => {
    const track = video.textTracks[0];
    const on = cc.getAttribute("aria-pressed") !== "true";
    cc.setAttribute("aria-pressed", String(on));
    if (track) track.mode = on ? "showing" : "hidden";
  });
  mute.addEventListener("click", () => {
    video.muted = !video.muted;
    mute.setAttribute("aria-pressed", String(video.muted));
    mute.setAttribute("aria-label", video.muted ? "Unmute" : "Mute");
  });
  full.addEventListener("click", () => {
    if (document.fullscreenElement) return document.exitFullscreen();
    if (player.requestFullscreen) player.requestFullscreen();
    else if (video.webkitEnterFullscreen) video.webkitEnterFullscreen(); // iPhone Safari
  });
})();
