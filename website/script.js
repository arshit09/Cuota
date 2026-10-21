// Cuota's website: pixel Clawd, the hero's live panel, Clawd loose on the page, and Cuota's
// 8-bit sounds. Sprites, words, the spinner and the sounds are ported from the app's src/.
(() => {
  "use strict";

  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const pick = (list) => list[Math.floor(Math.random() * list.length)];
  const rand = (min, max) => min + Math.random() * (max - min);

  /* ---------- Pixel art ---------- */

  // Clawd on a grid of 11 columns by 8 rows, from src/mascot.rs. '#' body, 'o' open eye,
  // '-' closed eye.
  const STAND = [
    ".#########.",
    ".#########.",
    "##o#####o##",
    "###########",
    ".#########.",
    ".#########.",
    ".#.#...#.#.",
    ".#.#...#.#.",
  ];
  const SPRITES = {
    stand: STAND,
    leftArmUp: [
      "##########.",
      "##########.",
      ".#o#####o#.",
      ".#########.",
      ".##########",
      ".##########",
      ".#.#...#.#.",
      ".#.#...#.#.",
    ],
    rightArmUp: [
      ".##########",
      ".##########",
      ".#o#####o#.",
      ".#########.",
      "##########.",
      "##########.",
      ".#.#...#.#.",
      ".#.#...#.#.",
    ],
    asleep: [
      "...........",
      "...........",
      ".#########.",
      ".#########.",
      "##-#####-##",
      "###########",
      ".#########.",
      ".#########.",
    ],
    asleepInhale: [
      "...........",
      "..#######..",
      ".#########.",
      ".#########.",
      "##-#####-##",
      "###########",
      ".#########.",
      ".#########.",
    ],
    // Mid-hop, his feet apart.
    step: [...STAND.slice(0, 6), ".#.#...#.#.", "#.#.....#.#"],
  };

  const SNORE_Z = ["####", "..#.", ".#..", "####"];

  const ICONS = {
    download: ["...###...", "...###...", "...###...", "#########", ".#######.", "..#####..", "...###...", "....#....", ".........", "#########"],
    menu: ["#########", "#########", ".........", ".........", "#########", "#########", ".........", ".........", "#########", "#########"],
    clock: ["..#####..", ".#.....#.", "#...#...#", "#...#...#", "#...###.#", "#.......#", "#.......#", ".#.....#.", "..#####.."],
    calendar: [".#.....#.", "#########", "#########", "#.......#", "#.#.#.#.#", "#.......#", "#.#.#.#.#", "#.......#", "#########"],
    chip: [".#.#.#.#.", "#########", "#.......#", "##.###.##", "#..###..#", "##.###.##", "#.......#", "#########", ".#.#.#.#."],
    ontop: ["#########", "#########", "#.......#", "#...#...#", "#..###..#", "#.#.#.#.#", "#...#...#", "#.......#", "#########"],
    desktop: ["#########", "#.....###", "#.....###", "#.......#", "#.......#", "#########", "....#....", "..#####.."],
    taskbar: ["#########", "#.......#", "#.......#", "#.......#", "#########", "#.#.#.###", "#########"],
    play: ["#......", "###....", "#####..", "#######", "#####..", "###....", "#......"],
    check: ["........#", ".......##", "#.....##.", "##...##..", ".##.##...", "..###....", "...#....."],
    file: ["#####...", "#...##..", "#...#.#.", "#...####", "#......#", "#......#", "#......#", "########"],
    key: [".###.....", "#...#....", "#...#####", "#...#.#.#", ".###..#.."],
  };

  // A grid's cells matching `match`, as one SVG path of unit squares, runs merged along rows.
  function gridPath(rows, match) {
    let d = "";
    rows.forEach((row, y) => {
      for (let x = 0; x < row.length; x++) {
        if (!match(row[x])) continue;
        let end = x;
        while (end + 1 < row.length && match(row[end + 1])) end++;
        d += `M${x} ${y}h${end - x + 1}v1h${x - end - 1}z`;
        x = end;
      }
    });
    return d;
  }

  function svg(width, height, unit, paths) {
    return `<svg viewBox="0 0 ${width} ${height}" width="${width * unit}" height="${height * unit}" shape-rendering="crispEdges" aria-hidden="true" focusable="false">${paths}</svg>`;
  }

  function iconSVG(rows, unit) {
    return svg(rows[0].length, rows.length, unit, `<path fill="currentColor" d="${gridPath(rows, (c) => c === "#")}"/>`);
  }

  // Clawd's sprite, his open eyes moved a cell by `look` to look about, or shut by `blink`.
  function clawdSVG(rows, unit, { look = [0, 0], blink = false } = {}) {
    const body = gridPath(rows, (c) => c !== ".");
    let eyes = "";
    rows.forEach((row, y) => {
      [...row].forEach((c, x) => {
        if (c === "o" && !blink) eyes += `M${x + look[0]} ${y + look[1]}h1v1h-1z`;
        else if (c === "o" || c === "-") eyes += `M${x} ${y + 0.5}h1v.5h-1z`;
      });
    });
    return svg(11, 8, unit, `<path class="clawd-body" d="${body}"/><path class="clawd-eye" d="${eyes}"/>`);
  }

  // Static art: <span class="sprite" data-sprite="stand" data-unit="3"> and
  // <span class="px-icon" data-icon="download">.
  $$("[data-sprite]").forEach((el) => {
    el.innerHTML = clawdSVG(SPRITES[el.dataset.sprite], Number(el.dataset.unit || 4));
  });
  $$("[data-icon]").forEach((el) => {
    const rows = ICONS[el.dataset.icon];
    if (!rows) return;
    const unit = Number(el.dataset.unit || (el.classList.contains("card-icon") ? 4 : el.closest(".sfx-play") ? 3 : 2));
    el.innerHTML = iconSVG(rows, unit);
  });

  // Clawd, posed and redrawn only when something changes.
  class Clawd {
    constructor(el, unit) {
      this.el = el;
      this.unit = unit;
      this.state = { pose: "stand", look: [0, 0], blink: false };
      this.key = "";
      this.draw();
    }

    set(changes) {
      Object.assign(this.state, changes);
      this.draw();
    }

    draw() {
      const { pose, look, blink } = this.state;
      const key = `${pose}|${look}|${blink}`;
      if (key === this.key) return;
      this.key = key;
      this.el.innerHTML = clawdSVG(SPRITES[pose], this.unit, { look, blink });
    }
  }

  // Which way Clawd at `rect` should look to see the pointer at `x`, `y`.
  function lookAt(rect, x, y) {
    const dx = x - (rect.left + rect.width / 2);
    const dy = y - (rect.top + rect.height * 0.35);
    return [Math.abs(dx) < rect.width * 0.4 ? 0 : Math.sign(dx), dy < -rect.height * 0.6 ? -1 : dy > rect.height ? 1 : 0];
  }

  const pointer = { x: -1, y: -1, seen: false };
  addEventListener("pointermove", (e) => {
    pointer.x = e.clientX;
    pointer.y = e.clientY;
    pointer.seen = true;
  }, { passive: true });

  function snore(fx, unit, spots) {
    spots.forEach(([left, top], i) => {
      const z = document.createElement("span");
      z.className = "snore";
      z.style.left = left;
      z.style.top = top;
      z.style.setProperty("--d", `${i * 0.9}s`);
      z.innerHTML = iconSVG(SNORE_Z, unit);
      fx.append(z);
    });
  }

  /* ---------- The status line ---------- */

  // As src/status.rs: a star that turns while Claude works, then what Clawd's up to. A new
  // word types itself over the last, a cursor sweeping across with the letters just behind it
  // still settling.
  const STARS = ["·", "✢", "*", "✶", "✻", "✽"];
  // The star at rest, as Claude Code leaves it once a turn is over.
  const RESTING = "✻";
  const STAR_MS = 120;
  const LETTER_MS = 40;
  const SETTLING = 2;
  const NOISE = [".", "_"];

  // The star showing `turned` ms into turning: up through STARS and back down.
  function star(turned) {
    const step = Math.floor(turned / STAR_MS) % (2 * STARS.length);
    return STARS[step < STARS.length ? step : 2 * STARS.length - 1 - step];
  }

  function typed(from, to, elapsed) {
    from = [...from];
    to = [...to];
    const cursor = Math.floor(elapsed / LETTER_MS);
    if (cursor > Math.max(from.length, to.length)) return null;
    const before = to.slice(0, cursor).map((c, i) => {
      if (c === " " || cursor - i > SETTLING) return c;
      // Each settling letter picks noise, or itself, afresh with each step of the cursor.
      let mixed = (Math.imul(i, 0x9e3779b9) ^ Math.imul(cursor, 0x85ebca6b)) >>> 0;
      mixed = (mixed ^ (mixed >>> 15)) >>> 0;
      return mixed % 3 === 0 ? c : NOISE[(mixed >>> 4) % NOISE.length];
    });
    // The cursor stands on the old letter it's replacing.
    return [before.join(""), from.slice(cursor + 1).join("")];
  }

  class StatusLine {
    constructor(el) {
      this.el = el;
      this.star = $(".star", el);
      this.text = $(".word", el);
      this.word = "";
      this.typing = null;
      this.lively = true;
      this.cursor = document.createElement("span");
      this.cursor.className = "type-cursor";
    }

    // Shows `word`, typed over the last one when `animate`. While `lively` (about work, or
    // celebrating) the star is orange and the word bright; otherwise both are dim.
    set(word, lively, now, animate) {
      if (word !== this.word) {
        const last = this.word;
        this.word = word;
        this.typing = animate && last ? { last, since: now } : null;
        if (!this.typing) this.text.textContent = word;
      }
      this.lively = lively;
      this.el.classList.toggle("dim", !lively);
    }

    // The star turns while lively, if `animate`; otherwise it rests.
    paint(now, animate) {
      const glyph = this.lively && animate ? star(now) : RESTING;
      if (this.star.textContent !== glyph) this.star.textContent = glyph;
      if (!this.typing) return;
      const text = typed(this.typing.last, this.word, now - this.typing.since);
      if (text) {
        this.text.replaceChildren(text[0], this.cursor, text[1]);
      } else {
        this.typing = null;
        this.text.textContent = this.word;
      }
    }
  }

  /* ---------- Words, from src/animations.rs ---------- */

  const WORDS = {
    busy: [
      "Accomplishing", "Actioning", "Alchemizing", "Baking", "Bedazzling", "Booping", "Brainstorming",
      "Brewing", "Calculating", "Cerebrating", "Channeling", "Churning", "Clauding", "Coalescing",
      "Cogitating", "Combobulating", "Computing", "Concocting", "Conjuring", "Considering",
      "Contemplating", "Cooking", "Crafting", "Creating", "Crunching", "Deciphering", "Deliberating",
      "Determining", "Discombobulating", "Distilling", "Divining", "Doodling", "Elucidating",
      "Enchanting", "Envisioning", "Fermenting", "Fiddling", "Finagling", "Flibbertigibbeting",
      "Forging", "Frolicking", "Galumphing", "Germinating", "Hatching", "Herding", "Honking",
      "Hustling", "Ideating", "Imagining", "Incubating", "Inferring", "Jiving", "Juggling", "Kneading",
      "Manifesting", "Marinating", "Meandering", "Mosaicing", "Moseying", "Mulling", "Musing",
      "Mustering", "Noodling", "Percolating", "Perusing", "Philosophizing", "Pondering",
      "Pontificating", "Processing", "Pursuing", "Puttering", "Puzzling", "Recombobulating",
      "Reticulating", "Riffing", "Ruminating", "Rummaging", "Scheming", "Schlepping", "Sculpting",
      "Shimmying", "Simmering", "Sleuthing", "Smooshing", "Spelunking", "Spitballing", "Stewing",
      "Sussing", "Synthesizing", "Thinking", "Tinkering", "Transmuting", "Unfurling", "Unraveling",
      "Untangling", "Vibing", "Weaving", "Whirring", "Whittling", "Wibbling", "Wizarding", "Wondering",
      "Working", "Wrangling",
    ],
    waiting: [
      "Waiting", "Idling", "Standing by", "Lounging", "Loafing", "Lollygagging", "Dawdling",
      "Daydreaming", "Twiddling thumbs", "Loitering", "Chilling",
    ],
    asleep: ["Sleeping", "Snoozing", "Dozing", "Napping", "Hibernating", "Slumbering", "Snoring"],
    done: ["Done", "All done", "Ta-da", "Nailed it", "Finished", "Shipped"],
    wander: ["Wandering", "Strolling", "Moseying", "Scuttling", "Pottering", "Roaming", "Exploring", "Sauntering"],
    hello: ["Hi", "Hello", "Hey there", "Hiya", "Howdy"],
    wake: ["Waking up", "Yawning", "Stretching"],
  };

  function another(list, not) {
    let word = pick(list);
    while (list.length > 1 && word === not) word = pick(list);
    return word;
  }

  /* ---------- The hero's live panel ---------- */

  // Clawd's slot in the panel, in points, how far a clip may spill out of it, and points per
  // GIF pixel at 100%, from src/animations.rs.
  const SLOT = [114, 96];
  const SPILL = 6;
  const GIF_SCALE = 0.3;

  // Plays Clawd's clips from the sheets in assets/clawd (made from the app's GIFs by
  // make_clips.py) as src/animations.rs plays the GIFs: each clip's drawing centred in the slot.
  class ClipPlayer {
    constructor(canvas, clips) {
      this.canvas = canvas;
      this.context = canvas.getContext("2d");
      this.clips = clips;
      this.sheets = new Map();
      this.name = "";
      this.shown = "";
    }

    sheet(name) {
      if (!this.sheets.has(name)) {
        const image = new Image();
        image.src = `assets/clawd/${name}.png`;
        this.sheets.set(name, image);
      }
      return this.sheets.get(name);
    }

    // Starts `name`, looping for `spell` ms, or playing once without one.
    play(name, now, spell = 0) {
      this.name = name;
      this.clip = this.clips[name];
      this.image = this.sheet(name);
      this.length = this.clip.ends[this.clip.ends.length - 1];
      this.spell = spell;
      this.started = now;
      this.ready = false;
    }

    // When the clip gives way: the end of the loop running when its spell is up.
    ends() {
      return this.started + Math.max(1, Math.ceil(this.spell / this.length)) * this.length;
    }

    // Draws the frame showing at `now`, or the first when not `animate`. A clip starts once its
    // sheet is in; until then the last one holds. Returns whether it's showing.
    draw(now, animate) {
      if (!this.ready) {
        if (!this.image.complete || !this.image.naturalWidth) return false;
        this.ready = true;
        this.started = now;
      }
      const { clip } = this;
      const position = animate ? (now - this.started) % this.length : 0;
      let index = clip.ends.findIndex((end) => end > position);
      if (index < 0) index = clip.ends.length - 1;
      const { width, height } = this.canvas;
      const key = `${this.name}|${index}|${width}`;
      if (key === this.shown) return true;
      this.shown = key;
      // Device pixels per point, the canvas being the slot and its spill.
      const k = width / (SLOT[0] + 2 * SPILL);
      const zoom = GIF_SCALE * k;
      const [, , boundsWidth, boundsHeight] = clip.bounds;
      const ctx = this.context;
      ctx.clearRect(0, 0, width, height);
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(
        this.image,
        (index % clip.cols) * clip.w,
        Math.floor(index / clip.cols) * clip.h,
        clip.w,
        clip.h,
        (SPILL + SLOT[0] / 2) * k - (boundsWidth * zoom) / 2,
        (SPILL + SLOT[1] / 2) * k - (boundsHeight * zoom) / 2,
        boundsWidth * zoom,
        boundsHeight * zoom,
      );
      return true;
    }
  }

  function heroPanel() {
    const panel = $("#hero-panel");
    const data = window.CUOTA_CLIPS;
    if (!panel || !data) return;
    const canvas = $("canvas", panel);
    const player = new ClipPlayer(canvas, data.clips);
    const status = new StatusLine($(".status", panel));
    const animate = !reduceMotion;

    // Clawd::update's moods, with their spells cut short: 20 to 60 s an activity, and a minute
    // waiting before he sleeps, would show most visitors a single clip.
    const SPELLS = { working: [6000, 9000], waiting: [8000, 10000], asleep: [9000, 12000] };
    const MOOD_WORDS = { working: [WORDS.busy, "…"], waiting: [WORDS.waiting, "…"], asleep: [WORDS.asleep, "…"] };
    // Activities before the work in the demo is done.
    const ACTIVITIES = 4;
    let mood = "working";
    let once = false;
    let activities = 0;

    // A clip from `pool` and a word from `words`, other than the ones showing.
    const clipFrom = (pool) => {
      const names = data.pools[pool];
      let name;
      do name = pick(names); while (names.length > 1 && name === player.name);
      return name;
    };
    const wordFrom = ([words, end]) => {
      let word;
      do word = pick(words) + end; while (words.length > 1 && word === status.word);
      return word;
    };

    // Plays a clip from `pool`, once or for a spell of the mood, with a new word.
    function play(pool, playOnce, words, now) {
      once = playOnce;
      player.play(clipFrom(pool), now, playOnce ? 0 : rand(...SPELLS[mood]));
      status.set(wordFrom(words), mood === "working" || playOnce, now, animate);
    }

    // What comes when a clip gives way: another activity, or the next mood. Work ending earns
    // a celebration and work starting after a rest a greeting, each played once.
    function next(now) {
      if (once) {
        play(mood, false, MOOD_WORDS[mood], now);
      } else if (mood === "working" && ++activities >= ACTIVITIES) {
        mood = "waiting";
        play("done", true, [WORDS.done, "!"], now);
      } else if (mood === "waiting") {
        mood = "asleep";
        play("asleep", false, MOOD_WORDS.asleep, now);
      } else if (mood === "asleep") {
        mood = "working";
        activities = 0;
        play("greeting", true, MOOD_WORDS.working, now);
      } else {
        play(mood, false, MOOD_WORDS[mood], now);
      }
    }

    // A clock that stops while the panel is out of sight, so he picks up where he left off.
    let offset = 0;
    let pausedAt = performance.now();
    let running = false;
    let visible = false;
    const clock = () => (running ? performance.now() : pausedAt) - offset;

    const fit = () => {
      const rect = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.max(1, Math.round(rect.width * dpr));
      canvas.height = Math.max(1, Math.round(rect.height * dpr));
      player.shown = "";
    };

    const frame = () => {
      if (!running) return;
      const now = clock();
      if (player.ready && now >= player.ends()) next(now);
      player.draw(now, true);
      status.paint(now, true);
      requestAnimationFrame(frame);
    };

    const setRunning = (on) => {
      if (on === running) return;
      if (on) {
        offset += performance.now() - pausedAt;
        running = true;
        requestAnimationFrame(frame);
      } else {
        pausedAt = performance.now();
        running = false;
      }
    };

    fit();
    play("working", false, MOOD_WORDS.working, clock());
    new ResizeObserver(() => {
      fit();
      player.draw(clock(), animate);
    }).observe(canvas);

    if (!animate) {
      // Held still, as with Animate Clawd off: the clip's first frame, the star at rest.
      status.paint(clock(), false);
      const still = () => {
        if (!player.draw(clock(), false)) requestAnimationFrame(still);
      };
      still();
      return;
    }

    new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      setRunning(visible && !document.hidden);
    }).observe(panel);
    document.addEventListener("visibilitychange", () => setRunning(visible && !document.hidden));
    // The other clips, ready before he gets to them.
    addEventListener("load", () => Object.keys(data.clips).forEach((name) => player.sheet(name)));
  }

  /* ---------- Clawd, loose on the page ---------- */

  const PET_KEY = "cuota-site-pet";

  function sitePet() {
    const root = $("#site-pet");
    if (!root) return;
    const body = $(".pet-body", root);
    const sprite = $(".pet-sprite", root);
    const bubble = $(".pet-bubble", root);
    const fx = $(".pet-fx", root);
    const toggles = $$("[data-pet-toggle]");
    const clawd = new Clawd(sprite, 4);
    const WIDTH = 44;
    const STEP_MS = 170;

    let x = Math.max(16, innerWidth * rand(0.15, 0.7));
    let dir = Math.random() < 0.5 ? -1 : 1;
    let mood = "walk";
    let until = 0;
    let tick = 0;
    let timer = 0;
    let bubbleUntil = 0;

    function say(text, ms = 2200) {
      bubble.textContent = text;
      bubble.hidden = false;
      bubbleUntil = performance.now() + ms;
    }

    function become(next, now) {
      mood = next;
      tick = 0;
      fx.replaceChildren();
      sprite.classList.remove("hop");
      if (next === "walk") {
        until = now + rand(3500, 9000);
        if (Math.random() < 0.35) dir = -dir;
        if (Math.random() < 0.4) say(`${pick(WORDS.wander)}…`);
      } else if (next === "idle") {
        until = now + rand(2500, 6000);
      } else if (next === "sleep") {
        until = now + rand(12000, 22000);
        snore(fx, 2, [["0px", "-6px"], ["10px", "-16px"]]);
      } else if (next === "wake") {
        until = now + 1400;
        say(`${pick(WORDS.wake)}…`, 1400);
      } else if (next === "hi") {
        until = now + 1700;
        say(`${pick(WORDS.hello)}!`, 1700);
      }
    }

    function step() {
      const now = performance.now();
      tick++;
      if (now > until) {
        if (mood === "walk") become(Math.random() < 0.3 ? "sleep" : "idle", now);
        else if (mood === "idle") become(Math.random() < 0.25 ? "sleep" : "walk", now);
        else if (mood === "sleep") become("wake", now);
        else become("walk", now);
      }
      let pose = "stand";
      let look = [0, 0];
      let blink = false;
      let hop = false;
      if (mood === "walk") {
        // Pixel-game hops: a cell up with his feet apart, then down.
        hop = tick % 2 === 1;
        pose = hop ? "step" : "stand";
        look = [dir, 0];
        x += dir * 7;
        const max = innerWidth - WIDTH - 12;
        if (x < 12 || x > max) {
          x = Math.min(Math.max(x, 12), max);
          dir = -dir;
        }
      } else if (mood === "idle") {
        if (pointer.seen) look = lookAt(body.getBoundingClientRect(), pointer.x, pointer.y);
        blink = tick % 22 === 21;
      } else if (mood === "sleep") {
        pose = Math.floor(tick / 5) % 2 ? "asleepInhale" : "asleep";
      } else if (mood === "wake") {
        pose = "stand";
        blink = tick % 4 < 2;
      } else if (mood === "hi") {
        pose = tick % 4 < 2 ? "leftArmUp" : "stand";
      }
      if (reduceMotion) hop = false;
      clawd.set({ pose, look, blink });
      sprite.classList.toggle("hop", hop);
      root.style.transform = `translateX(${Math.round(x)}px)`;
      if (!bubble.hidden && now > bubbleUntil) bubble.hidden = true;
    }

    function run() {
      clearInterval(timer);
      timer = 0;
      if (root.hidden || document.hidden) return;
      if (reduceMotion) {
        // He naps where he is; a click still wakes him for a hello.
        if (mood === "walk") become("sleep", performance.now());
        mood = mood === "hi" ? mood : "sleep";
        until = Infinity;
        step();
        return;
      }
      step();
      timer = setInterval(step, STEP_MS);
    }

    function show(on, remember) {
      root.hidden = !on;
      toggles.forEach((button) => {
        button.textContent = on ? "Put Clawd away" : "Let Clawd out";
        button.setAttribute("aria-pressed", String(on));
      });
      if (remember) {
        try {
          localStorage.setItem(PET_KEY, on ? "1" : "0");
        } catch {}
      }
      if (on) {
        x = Math.min(x, innerWidth - WIDTH - 12);
        become(reduceMotion ? "sleep" : "walk", performance.now());
        if (remember) say(`${pick(WORDS.hello)}!`);
      }
      run();
    }

    body.addEventListener("click", () => {
      become("hi", performance.now());
      if (reduceMotion) {
        step();
        setTimeout(() => {
          bubble.hidden = true;
          become("sleep", performance.now());
          until = Infinity;
          step();
        }, 1700);
      }
    });
    toggles.forEach((button) => button.addEventListener("click", () => show(root.hidden, true)));
    document.addEventListener("visibilitychange", run);

    let saved = null;
    try {
      saved = localStorage.getItem(PET_KEY);
    } catch {}
    show(saved === null ? innerWidth >= 720 : saved === "1", false);
  }

  /* ---------- Sounds, from src/sound.rs ---------- */

  // NES-style channels: a pulse wave's duty as a number, or the triangle channel.
  const SQUARE = 0.5;
  const PULSE = 0.25;
  const TRIANGLE = "triangle";

  const RATE = 44100;
  const ATTACK = 0.003;
  const HOLD = 0.3;
  const VIBRATO_HZ = 6;
  const SOFTEN_HZ = 6000;
  const TAIL = 0.03;
  const TAU = Math.PI * 2;

  const E3 = 164.81, F3 = 174.61, B3 = 246.94, C5 = 523.25, A5 = 880.0, C6 = 1046.5, E6 = 1318.51, G6 = 1567.98;
  const A6 = 1760.0, C7 = 2093.0;

  const note = (wave, hz, at, length, { slide = hz, level = 1, vibrato = 0 } = {}) =>
    ({ wave, from: hz, to: slide, at, length, level, vibrato });

  const TUNES = {
    start: { peak: 0.13, notes: [note(PULSE, C6, 0, 0.05, { slide: E6 }), note(PULSE, G6, 0.055, 0.1)] },
    done: {
      peak: 0.14,
      notes: [
        note(SQUARE, C6, 0, 0.055),
        note(SQUARE, E6, 0.06, 0.055),
        note(SQUARE, G6, 0.12, 0.055),
        note(SQUARE, C7, 0.18, 0.24, { level: 0.7, vibrato: 0.012 }),
        note(TRIANGLE, C5, 0.18, 0.24),
      ],
    },
    error: { peak: 0.1, notes: [note(SQUARE, B3, 0, 0.09), note(SQUARE, F3, 0.11, 0.2, { slide: E3, vibrato: 0.02 })] },
    ask: {
      peak: 0.13,
      notes: [note(PULSE, A5, 0, 0.05), note(PULSE, A5, 0.09, 0.05), note(SQUARE, E6, 0.2, 0.18, { slide: A6, level: 0.8 })],
    },
  };

  // A wave's level at `phase`, from 0 to 1 through a cycle.
  function waveAt(wave, phase) {
    if (wave === TRIANGLE) {
      // Down from 15 to 0 and back up, in 32 steps a cycle.
      const step = Math.floor(phase * 32);
      const level = step < 16 ? 15 - step : step - 16;
      return level / 7.5 - 1;
    }
    return 2 * (phase < wave ? 1 - wave : -wave);
  }

  // Up in an instant, held a while, then fading away, in the NES's 16 steps of volume.
  function envelope(t, length) {
    const rise = Math.min(t / ATTACK, 1);
    const fall = Math.min(Math.max((1 - t / length) / (1 - HOLD), 0), 1);
    return Math.round(rise * fall * 15) / 15;
  }

  function render(tune) {
    const end = Math.max(...tune.notes.map((n) => n.at + n.length)) + TAIL;
    const mix = new Float32Array(Math.ceil(end * RATE));
    for (const n of tune.notes) {
      let phase = 0;
      const start = Math.floor(n.at * RATE);
      const count = Math.min(Math.floor(n.length * RATE), mix.length - start);
      for (let i = 0; i < count; i++) {
        const t = i / RATE;
        const wobble = 1 + n.vibrato * Math.sin(TAU * VIBRATO_HZ * t);
        const hz = n.from * Math.pow(n.to / n.from, t / n.length) * wobble;
        mix[start + i] += n.level * envelope(t, n.length) * waveAt(n.wave, phase);
        phase = (phase + hz / RATE) % 1;
      }
    }
    // Softened above SOFTEN_HZ, as by a TV's speaker, then scaled to the tune's peak.
    const smoothing = 1 - Math.exp((-TAU * SOFTEN_HZ) / RATE);
    let soft = 0;
    let loudest = 0;
    for (let i = 0; i < mix.length; i++) {
      soft += smoothing * (mix[i] - soft);
      mix[i] = soft;
      loudest = Math.max(loudest, Math.abs(soft));
    }
    const gain = loudest > 0 ? tune.peak / loudest : 0;
    for (let i = 0; i < mix.length; i++) mix[i] *= gain;
    return mix;
  }

  let audio = null;
  const buffers = new Map();

  function play(tune) {
    audio ??= new (window.AudioContext || window.webkitAudioContext)();
    if (audio.state === "suspended") audio.resume();
    let buffer = buffers.get(tune);
    if (!buffer) {
      const samples = render(tune);
      buffer = audio.createBuffer(1, samples.length, RATE);
      buffer.getChannelData(0).set(samples);
      buffers.set(tune, buffer);
    }
    const source = audio.createBufferSource();
    source.buffer = buffer;
    source.connect(audio.destination);
    source.start();
    return buffer.duration * 1000;
  }

  function sounds() {
    $$(".sfx").forEach((button) => {
      button.addEventListener("click", () => {
        let ms = 300;
        try {
          ms = play(TUNES[button.dataset.sound]);
        } catch {
          return;
        }
        button.classList.add("playing");
        clearTimeout(button.playing);
        button.playing = setTimeout(() => button.classList.remove("playing"), Math.max(ms, 240));
      });
    });
  }

  /* ---------- The rest of the page ---------- */

  function navMenu() {
    const nav = $(".nav");
    const toggle = $(".menu-toggle");
    if (!toggle) return;
    const set = (open) => {
      nav.classList.toggle("open", open);
      toggle.setAttribute("aria-expanded", String(open));
    };
    toggle.addEventListener("click", () => set(!nav.classList.contains("open")));
    $$(".nav-links a").forEach((a) => a.addEventListener("click", () => set(false)));
    addEventListener("keydown", (e) => {
      if (e.key === "Escape") set(false);
    });
  }

  function reveals() {
    const items = $$(".reveal");
    if (reduceMotion || !("IntersectionObserver" in window)) {
      items.forEach((el) => el.classList.add("in"));
      return;
    }
    const seen = new IntersectionObserver((entries) => {
      entries
        .filter((entry) => entry.isIntersecting)
        .forEach((entry, i) => {
          entry.target.style.transitionDelay = `${i * 90}ms`;
          entry.target.classList.add("in");
          seen.unobserve(entry.target);
        });
    }, { rootMargin: "0px 0px -8% 0px" });
    items.forEach((el) => seen.observe(el));
  }

  // The Clawd by the last download button waves while he's on screen.
  function wave() {
    const el = $(".final-clawd");
    if (!el || reduceMotion) return;
    const clawd = new Clawd(el, Number(el.dataset.unit));
    let timer = 0;
    new IntersectionObserver(([entry]) => {
      clearInterval(timer);
      if (!entry.isIntersecting) return;
      let up = false;
      timer = setInterval(() => clawd.set({ pose: (up = !up) ? "leftArmUp" : "stand" }), 420);
    }).observe(el);
  }

  function ticker() {
    const track = $(".ticker-track");
    if (!track || reduceMotion) return;
    // Two copies, so sliding by half loops without a seam.
    track.append(...[...track.children].map((child) => child.cloneNode(true)));
  }

  // The newest release's version and size, from GitHub. The page shows v1.1.0 until then.
  async function release() {
    try {
      const response = await fetch("https://api.github.com/repos/arshit09/Cuota/releases/latest", {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!response.ok) return;
      const latest = await response.json();
      const exe = (latest.assets || []).find((asset) => asset.name === "Cuota.exe");
      if (latest.tag_name) $$("[data-release-version]").forEach((el) => (el.textContent = latest.tag_name));
      if (exe) {
        const size = `${(exe.size / 1048576).toFixed(1)} MB`;
        $$("[data-release-size]").forEach((el) => (el.textContent = size));
      }
    } catch {}
  }

  heroPanel();
  sitePet();
  sounds();
  navMenu();
  reveals();
  wave();
  ticker();
  release();
})();
