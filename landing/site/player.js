// The hero player. At rest the turntable loops (the record turns on its own); hovering or focusing
// the play button on the vinyl swaps in the loop where the button lifts and glows; a click plays the
// film in the same frame, with the designed controls. Until the film is published (/media/demo.mp4)
// the click says so instead of opening an empty player.
(() => {
  const player = document.querySelector(".player");
  if (!player) return;
  const rest = player.querySelector(".loop.rest");
  const lift = player.querySelector(".loop.lift");
  const film = player.querySelector(".film");
  const play = player.querySelector(".play");
  const soon = player.querySelector(".soon");
  const now = player.querySelector(".now");
  const durations = player.querySelectorAll(".duration");
  const progress = player.querySelector(".progress");
  const played = player.querySelector(".played");
  const cc = player.querySelector(".cc");
  const mute = player.querySelector(".mute");
  const full = player.querySelector(".full");
  const src = player.dataset.src;
  const still = window.matchMedia("(prefers-reduced-motion: reduce)");

  const clock = (s) => {
    s = Math.max(0, Math.floor(s || 0));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  const state = (s) => { player.dataset.state = s; };
  const atRest = () => ["rest", "lift", "soon"].includes(player.dataset.state);

  // The loops: muted, so browsers let them start on their own; reduced motion keeps their stills.
  const spin = (video) => {
    if (still.matches) return;
    const p = video.play();
    if (p) p.catch(() => {});
  };
  spin(rest);

  // The two loops are the same 3.6 s turn: the lifted one takes over at the same point of it.
  const hover = (on) => {
    if (!atRest()) return;
    const [from, to] = on ? [rest, lift] : [lift, rest];
    if (Number.isFinite(from.currentTime)) to.currentTime = from.currentTime;
    spin(to);
    from.pause();
    state(on ? "lift" : "rest");
  };
  play.addEventListener("pointerenter", () => hover(true));
  play.addEventListener("pointerleave", () => hover(false));
  play.addEventListener("focus", () => hover(true));
  play.addEventListener("blur", () => hover(false));

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
        soon.textContent = "THE FILM LANDS SOON";
        state("soon");
        return;
      }
      if (!film.currentSrc) {
        film.src = src;
        film.preload = "auto";
      }
      rest.pause();
      lift.pause();
      state("playing");
      return film.play().catch(() => state("paused"));
    });

  play.addEventListener("click", start);
  film.addEventListener("click", () => (film.paused ? film.play() : film.pause()));
  film.addEventListener("play", () => state("playing"));
  film.addEventListener("pause", () => { if (!film.ended) state("paused"); });
  // At the end the turntable comes back, turning.
  film.addEventListener("ended", () => {
    state("rest");
    spin(rest);
  });
  film.addEventListener("loadedmetadata", () => {
    durations.forEach((d) => (d.textContent = clock(film.duration)));
  });
  film.addEventListener("timeupdate", () => {
    const p = film.duration ? (film.currentTime / film.duration) * 100 : 0;
    now.textContent = clock(film.currentTime);
    played.style.setProperty("--p", `${p}%`);
    progress.setAttribute("aria-valuenow", String(Math.round(p)));
  });

  const seek = (fraction) => {
    if (!film.duration) return;
    film.currentTime = Math.min(1, Math.max(0, fraction)) * film.duration;
  };
  progress.addEventListener("click", (e) => {
    const r = progress.getBoundingClientRect();
    seek((e.clientX - r.left) / r.width);
  });
  progress.addEventListener("keydown", (e) => {
    if (!film.duration) return;
    const step = e.key === "ArrowRight" ? 5 : e.key === "ArrowLeft" ? -5 : 0;
    if (step) {
      e.preventDefault();
      film.currentTime = Math.min(film.duration, Math.max(0, film.currentTime + step));
    }
  });

  cc.addEventListener("click", () => {
    const track = film.textTracks[0];
    const on = cc.getAttribute("aria-pressed") !== "true";
    cc.setAttribute("aria-pressed", String(on));
    if (track) track.mode = on ? "showing" : "hidden";
  });
  mute.addEventListener("click", () => {
    film.muted = !film.muted;
    mute.setAttribute("aria-pressed", String(film.muted));
    mute.setAttribute("aria-label", film.muted ? "Unmute" : "Mute");
  });
  full.addEventListener("click", () => {
    if (document.fullscreenElement) return document.exitFullscreen();
    if (player.requestFullscreen) player.requestFullscreen();
    else if (film.webkitEnterFullscreen) film.webkitEnterFullscreen(); // iPhone Safari
  });
})();
