<!-- CHANGELOG.md -->
# Changelog

What changed, by release. For **why**, see [`DECISIONS.md`](DECISIONS.md); for the full roadmap and verification status, [`PHASES.md`](PHASES.md).

Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

> **Versioning note:** only `v0.3.0-ai-worker` was ever tagged. The `v0.1.0` and `v0.2.0` entries below mark phases, not tags. The two halves also disagree — `mavis_core` says `0.1.0`, `mavis_worker` says `0.3.0`. Everything since the tag is listed under Unreleased.

---

## [Unreleased]

192 commits since `v0.3.0-ai-worker`, covering Phases 4 through 8.5.

### Added
- **Commands MAVIS works out itself:** for anything its actions and facts don't cover, the model writes a shell command, reads what it prints and answers from it, up to three commands per request. Commands that only read run without asking; anything else asks first, `sudo` goes through `pkexec`, and a command is stopped after 30 seconds. The hand-written volume, brightness, battery, what's-playing and clipboard lookups are gone. Volume, brightness, Bluetooth, Wi-Fi, media and workspace changes don't ask either.
- **"Close whatever you just opened"** closes the app MAVIS last opened. "Close the active window" works without the model.
- **Requests in your own words:** when no rule knows a request ("it's way too loud", "kill the browser", "go to my third desktop"), the model picks one of MAVIS's commands, or none. The pick goes through the same rules and confirmations as a spoken command; with none, MAVIS replies as before. Questions go straight to the reply.
- **Window and system controls by voice:** switch workspace, full screen, volume and brightness to a level, Bluetooth and Wi-Fi on or off, lock, log out, suspend, shut down and restart. The last four ask first.
- **Apps by what they are:** "open the terminal", "close the browser". MAVIS asks which when several fit.
- **`[stt]` in `config/config.toml`** — choose the Whisper model (e.g. the English-only `small.en`), device and compute type.
- **"Close <app>"** — closes that app's open windows after asking first, since unsaved work could go with them. The first voice command that reaches "Shall I…?". niri only for now.
- **Push to talk** — set `listen_mode = "push"` in `config/config.toml` to have MAVIS listen only after you ask. In push-to-talk mode, press your hotkey (`toggle_listen.sh`) or tap the orb, then speak; it hears one sentence and stops. Always listening stays the default. `MAVIS_LISTEN_MODE` overrides it.
- **Tap the orb to confirm** — a quick tap answers "Shall I run …?" with yes. Administrator actions still need the words.
- **Longer conversations hold together** — what you said before the last dozen turns reaches the model as one short line, instead of being forgotten.
- **Barge-in** — talk over MAVIS and it stops speaking immediately; say "stop" and it goes quiet without answering. It listens while it talks, against a threshold set above its own voice. `MAVIS_BARGE_IN=0` turns it off.
- **"Sorry, I didn't catch that"** when a transcript is too unreliable to act on, instead of answering a misheard question.
- **System Sentinel** (Phase 8.5, opt-in via `MAVIS_SENTINEL=1`) — reads the pacman, dpkg or dnf transaction log and notices packages you didn't ask for, removals and downgrades. First run imports history silently; one sentence per update; routine upgrades never mentioned. Pending changes are said the next time you speak; "what changed recently?" and "what did that update do?" are answered from its record, without the model. Also watches who can do what: new accounts, group membership, SSH keys, sudoers, enabled services and setuid binaries — gaining root-level access raises a desktop notification immediately. Once a day it checks that packaged files still match their packages (`pacman -Qkk`, `rpm -Va`, `dpkg --verify`) and, with `MAVIS_SENTINEL_ADVISORIES=1`, relays new high-risk advisories from `arch-audit` or `debsecan`. Windows and macOS sources (installed apps, startup items, Defender detections, Homebrew, launchd) are written but unverified.
- **Permission gate** (Phase 8) — every plan is scored 0–10 and recorded before the executor sees it. Silent below 3, "Shall I?" from 3, "yes, administrator" from 8, irreversible commands refused.
- **Append-only audit log** — `audit.db`, with no update or delete path.
- **Undo** — before a destructive command runs, the files it names are copied; "undo that" restores them within five minutes.
- **The confirmation question reads out the command**, and the orb turns orange while MAVIS waits for the answer.
- **Subsystem supervision** — a panicking subsystem is logged and restarted instead of silently disappearing.
- **Memory** (Phase 7) — FTS5 recall with importance-based decay, hourly daily-summary consolidation, time-range replay ("what was I doing yesterday afternoon"), and an entity graph of projects, apps and files.
- **Actions** (Phase 6.5) — launch any installed app, YouTube and web search, URLs, and voice control of volume, media and brightness. Apps discovered from `.desktop` files, Start Menu shortcuts or `.app` bundles.
- **Context awareness** (Phase 6) — active window, open windows and workspaces, clipboard, IDE and terminal awareness, git project and branch, calendar, date and time. Each source opt-in via `MAVIS_CONTEXT_*`.
- **Voice** (Phases 4–5) — faster-whisper STT, Piper and Kokoro TTS, a non-blocking TTS queue with interruption, session recovery across restarts, and the celebrating orb state.
- `MAVIS_VAD_DEBUG` to see what the microphone is actually producing.
- `MAVIS_ORB=off` and `MAVIS_ORB_POS=x,y`.

### Changed
- **No playerctl needed:** MAVIS asks media players what's playing, and plays, pauses or skips, over D-Bus itself.
- **Steadier answers:** MAVIS sees its last few turns in full and older ones as notes, so one bad reply doesn't spread. It knows the battery level and can tell you what's playing.
- **Context and the Sentinel are switched in `config/config.toml`,** not per-run variables; desktop awareness and the Sentinel are on as shipped. **Faster replies:** the tool list is cached between calls and speech recognition does less work. The clipboard is read only when you ask, through a lookup.
- **Actions work again** (tool calls were being cut off and read aloud), **questions about system changes and your past are looked up**, and MAVIS no longer mistakes its own voice for yours when it hears it slightly wrong.
- **MAVIS acts through its model's own tool calls** and tells you what happened after it's done, in its own words. Replies are full sentences again.
- **Long conversations.** Qwen2.5-3B is now the default model and sees far more of the conversation; older parts are summarised instead of forgotten.
- **Questions, warnings and system news in MAVIS's own words,** and your name is learned from what you say, not from fixed phrases.
- **MAVIS says it in its own words.** Actions no longer come with fixed sentences: the model says what it's doing as it acts, and when something can't be done or fails, MAVIS tells you what happened in its own words.
- **Code comments without test history:** dates, run references and quoted transcripts moved out of the code. The reasons stay; the history lives in `DECISIONS.md`.
- **No phrase lists left.** MAVIS's answers to "Shall I…?", questions about system changes, "stop", looking back over past days and remembering facts all go through the model as actions (`confirm`, `cancel`, `system_changes`, `update_details`, `stop`, `look_back`, `remember`). The word lists, the Sentinel's question grammar, time-phrase parsing, the command tables, the reply filters and the real names in tests and comments are gone.
- **High-risk actions are asked twice,** the second time saying they can't be undone, instead of needing "yes, administrator". A tap on the orb never answers them.
- **Qwen models are supported:** a model file with "qwen" in its name uses its chat format, for comparing against Phi-3 at choosing actions.
- **MAVIS understands instead of matching phrases.** Every sentence goes to the model once, which answers `DO <action>` or `SAY <reply>`, enforced by a grammar. Actions come from a catalogue in code. The code checks each one before it runs: the action exists, levels are 0–100, and named apps or sites come from what you said. The request filter, the separate command chooser, the examples copied from logs and the command tables on the live path are gone. Simple commands now take about as long as a short reply.
- **The orb follows the cursor while you drag it**, like a window.
- **The orb floats.** On Wayland desktops with the layer shell (niri, Hyprland, Sway, KDE) it's an overlay above every window, on every workspace, never taking keyboard focus, and you can drag it anywhere. GNOME and X11 keep the window.
- **"Play X" works with any app**, not just a browser: browsers get the site from `[media]` in `config/config.toml`, media players get a matching file from your Music or Videos folder, and apps with their own link type get a search. The app you last played in is used next time. "Search for X" uses `[web]`. No site or app names in code.

### Fixed
- **Replies no longer narrate or echo you,** questions are answered instead of acted on, and an action can't be read out loud.
- **The model's actions were loose.** It wrote `volume_set <80>`, invented `volume_down 20`, and once produced `workspace 2. SAY "Thunar is now open…"`. The grammar now holds each argument to its kind: a level is a number from 0 to 100, a workspace a number, an app one of the installed or open apps, and a website a domain. `open_website` is separate from `open`. Choosing an action runs at temperature 0.4, and the instructions say questions get a SAY and requests get a DO.
- **MAVIS sat deaf without saying so** when the mic was too quiet. Bursts of sound that stay above the room but below the speech threshold, four times in 30 seconds with no speech, now get one spoken warning.
- **The Sentinel named packages twice** ("removed dux-gop, dux-shell, dux-gop and dux-shell"). Each package is described by what it ended up as. One installed and removed again in the same update isn't mentioned.
- **MAVIS interrupted itself** reading out "P_RSA3072SHA384.json": its own voice came back as "PRSA". A piece of a name it read out now counts as its own voice. A word it said whole, like "stop", still interrupts.
- **The Sentinel was hard to listen to:** file names a voice can't read are counted rather than read out. A second update on the same day is "Another update today…". After a "what changed" answer, the changes it counted are no longer read out again on the next sentence.
- **"Window manager, not the active window" was taken as "close this window".** Corrections skip the command choice, and the choice now sees the last exchange.
- **Made-up levels and shortcuts:** "your volume is at a comfortable level", "use Ctrl+Alt+Right". The model is told the real speaker volume and screen brightness, and doesn't give levels or shortcuts it wasn't told.
- **"Workspace too"** (Whisper's "two") switches to workspace 2. **"All the way up"** means 100.
- **Brightness needed `brightnessctl`.** Without it, MAVIS asks systemd-logind, which every systemd desktop has.
- **The mic warning suggested a fixed level (0.35)** that left MAVIS deaf on this machine. It now suggests a step down.
- "Is there something specific you'd like to do…?" is dropped like the other follow-up offers.
- **A false "new program that runs as root" alert** after libgtop's update. The update moved `libgtop_server2` to another folder. A setuid program with the same name, mode and owner that disappears from one folder and appears in another is now reported once, as a move, and spoken rather than raised as a critical alert.
- **Questions were acted on:** "No, what application is open on workspace one?" opened About Xfce, and "Can you explain the recent changes…?" opened a YouTube search. Questions now skip the command choice even after "No," or "OK", with a "?" at the end, or as "can you explain" or "I want to know". The model's pick is also ignored if it names an app or site nobody mentioned.
- **"Close Timeshift" said it wasn't open:** a window named after an app (`timeshift-gtk`) now belongs to it, even when the launcher runs another program.
- **"What are the vision changes on my system?"** (a mishearing of "recent") is answered by the Sentinel: one misheard word is let through.
- **MAVIS claimed to do things it hadn't** ("closing the terminal now", "Thunar is now full screen"). Such replies become "I can't do that yet", and a failed action is said out loud with the reason.
- **A misheard "close Thunar" could have closed the terminal MAVIS runs in.** It never closes its own window now.
- **MAVIS answered its own voice** picked up by the mic. Its recent words are ignored while it speaks.
- **Window lists read out IDs** like "org.kde.haruna". They now use app names, and "[mavis]" labels and follow-up questions are cleaned out of replies.
- **MAVIS said your name in almost every reply.** Now only the first reply after starting may use it.
- **Found in review before testing** (DECISIONS §36):
  - "Open tuner" now finds Thunar even with Bulk Rename installed.
  - "Yes Mavis, close it" confirms, and "close it" works while the orb has focus.
  - MAVIS starts offline again once its models are downloaded.
  - "Mavis" stays in the speech prompt even with many apps installed.
  - "Search the web for X" searches; "play X on YouTube" after a music app uses a browser.
  - Terminals and editors are never handed a search link; "Mavis?" gets "Yes?".
- **MAVIS heard nothing after switching to `small.en`.** The voice engine had put the worker offline, so the new speech model couldn't download. The worker now stays online until every model is downloaded, falls back to a speech model already on disk, and the core no longer retries a refused request five times.
- **MAVIS went quiet with a red orb when it couldn't hear.** It now says why ("my speech model is still loading"), and the red fades back after a few seconds.
- **Switching window or workspace pulled you back** to the window you'd left. MAVIS now moves focus off its orb only once, as it opens.
- **The speech model setting in `config/config.toml` was ignored**: the worker only looked in `~/.config/mavis`. It now falls back to the repo's file, whose model settings now match what actually runs.
- **"Open Thunar" opened Bulk Rename**, another launcher entry using the same program.
- **Answering "Shall I close…?" with "Yes Mavis, close …" cancelled.** It confirms now; "close it" closes the focused window.
- **MAVIS made up whether the system had changed or had updates.** It no longer guesses, and a misheard "what changed recently" still gets the real answer.
- **"Play X" only searched YouTube.** It now opens the top video, falling back to the search if YouTube can't be reached.
- **Apps with a misheard name didn't open** ("open tuner" for Thunar), and MAVIS claimed it couldn't open applications. Near misses match now, and a missing app is named as missing.
- **"Close Firefox" couldn't be confirmed** by saying it again, and short commands were dropped as "didn't catch that". Repeating the request confirms it, and recognised commands are never dropped.
- **MAVIS was heard as "Movies" and "Maybe".** The English-only speech model is the default, and Whisper is told how people address MAVIS.
- **"How many windows?" gave the number of apps**, "what am I looking at?" read out shell symbols, and "tell me the recent changes" got a promise instead of an answer.
- **MAVIS said it had no voice**, and sometimes narrated itself ("Mavis responds…") instead of answering. It now knows its replies are spoken and talks in the first person; narration is reduced to what it says.
- **Replies were clipped** to two sentences and 180 characters ("4 workspaces open."). Up to three full sentences now.
- **"Sorry, how was that?" got a different answer.** Asked to repeat, MAVIS repeats.
- **MAVIS spoke with the flat Piper voice** even though the natural Kokoro voice was loaded on every start: the executor only used Kokoro when told to. Kokoro is now the default, spoken a sentence at a time; Piper remains the fallback.
- **"What am I looking at?" named the wrong window** — the orb took keyboard focus when it opened, so MAVIS knew of no focused window and the model guessed. MAVIS now reads the window it was started from, and on niri hands focus back to it.
- **"Stop" while MAVIS was speaking was never heard** — a side effect of the stutter fix, which made a single word too short to count. MAVIS now listens at once and only pauses its voice for longer speech; a lone "stop" is transcribed and acted on.
- **"What's focused?" answered with an earlier answer** — the model copied its previous reply ("Brave") with kitty focused. What's on screen now sits apart from the conversation, last in the prompt.
- **MAVIS's voice cut out and stuttered.** Music under its voice, and brief sounds, read as you interrupting: 9 of 10 interruptions were noise. Media now pauses while MAVIS speaks, and an interruption takes 360 ms of speech.
- **"Stop" did nothing to music**, and a quiet "stop" was often discarded as unclear. It now pauses what's playing when MAVIS isn't speaking, and is always honoured.
- **"Play X on YouTube in Firefox" ignored Firefox**, and the search swallowed the rest of the sentence. The video now opens in the app named, and the search ends with the sentence.
- **"Open Firefox and play X" only opened Firefox.** Both requests run now.
- **"MOVIS, what changed recently?"** — a misheard name made MAVIS miss the question.
- **Wrong counts of open apps and workspaces.** MAVIS counts them itself now, and uses the workspace numbers the desktop shows.
- **Apps whose launcher quotes a path or argument didn't start** — `Exec="/opt/My App/run"`, Wine and JetBrains launchers, and `open -a "Visual Studio Code"` were split at every space.
- **A request made while MAVIS was asking "Shall I?" was lost** — it cancelled the question and then did nothing. It now cancels and still carries the request out.
- **"…and postfix and removed debsecan"** — the Sentinel's sentences no longer run two "and"s together.
- **The worker could unload a model in the middle of a reply** if the reply took longer than the idle timeout.
- **Ctrl+C left the worker running** after removing its socket.
- **`kill` lost the conversation** — only Ctrl+C saved working memory; SIGTERM now does too.
- **Dropped utterances are logged** when transcription is still busy.
- **A plan wrapped as `{"actions": [...]}` was asked about instead of refused** — `rm -rf /` in that shape needed only a "yes". The gate and executor now read plans the same way.
- **Commands launched as an "app" skipped risk scoring** — `sudo …` and `sh -c …` ran silently at risk 2.
- **"Okay, so what about…" counted as consent.** The whole answer now has to be a yes.
- **The answer to "Shall I?" was also sent to the model**, which would have replied to "yes".
- **The worker socket was open to every local user** (`0666`); now owner-only. Speech audio no longer goes through fixed names in `/tmp`.
- **"What's on my clipboard?" answered with a single word from a shell prompt** — a multi-line clipboard was pasted into the prompt raw, breaking it into fragments. It's now flattened to one line, with its true length stated when it's long.
- **Recall offering old commands as context** — only things you stated about yourself are put into prompts now; everything is still recorded for replay and summaries.
- **Audio arriving above full scale** (2.2× in recent runs) is scaled back before transcription instead of being passed on distorted.
- **A fresh start answered with the last session's conversation** — "can you hear me" got the clipboard contents from days earlier. A restart now begins a new conversation; your name, memories, summaries and projects still carry over.
- **Recall returning the question you just asked**, and questions recorded as memories by older builds — both now cleared at startup.
- **MAVIS cut itself off on every reply** when barge-in first met real speakers: it measured its own voice during the silence before playback started, so its first word looked like someone interrupting. It now measures while sound is actually playing, tracks the loudest recent playback rather than an average, and an interruption pauses the reply instead of killing it — if what it heard turns out to be its own voice, the reply carries on.
- **Capture now prefers PipeWire** (and an echo-cancelled source above everything) over the raw sound card, so system-level echo cancellation and the user's chosen source apply.
- **`MAVIS_AUDIO_DEVICE` broke playback** — it names a microphone and was being passed to the audio player. Output now has its own variable, `MAVIS_AUDIO_OUTPUT`.
- The worker now says so in the log when the microphone is clipping.
- **MAVIS said "Opening Firefox" without opening anything** — commands were only recognised at the start of a sentence, a comma after the wake word blocked them, and an app didn't match unless every spoken word was in its name. The model is also now told it cannot act, so it stops narrating actions.
- **Repeating its own last reply** on short or garbled input — less history in the prompt, and known facts now sit last where they outrank the conversation.
- **Real speech dropped** as "too short" (minimum 1.5 s → 0.8 s) or as room noise; the noise floor no longer drifts upward on word onsets and echo.
- **Questions stored as memories**, which fed the user's old questions back into every prompt.
- **The worker's own logs never appeared** — logging was configured but never switched on.
- Kokoro contacted Hugging Face on every start even with the model cached.
- **60-second replies** (found in the 2026-09-22 live run) — three causes, all fixed:
  - *Listening never ended in a noisy room.* The VAD's thresholds couldn't rise above fixed values, so fan noise held utterances open to the 45 s ceiling. The noise floor is now measured at startup, follows the room, survives between utterances, and is re-measured mid-utterance if the room gets louder; trailing noise is trimmed. Ceiling 45 s → 30 s.
  - *Whisper invented "Thank you for watching, please subscribe…"* after real speech. Silero VAD, already bundled with faster-whisper, now marks which audio is speech; Whisper hears only that and isn't called at all without it. The hallucination phrase lists are gone.
  - *The LLM generated up to 256 tokens and kept ~40.* Generation now streams and stops once the reply is complete; the system prompt is pre-evaluated while the user is still speaking.
- **Unpunctuated transcripts came back empty** — sentence de-duplication dropped text after the last punctuation mark.
- **"The user's name is Using"** — names are recognised by how they're said (explicit introduction, last in the clause, not a question), with no word lists; names stored by older builds are discarded once.
- **MAVIS saw its own orb as the active window**, and reported its own directory as the user's project.
- **Every reply was in the prompt twice**, and the current utterance could be too.
- **Clipboard sent with every message** — now only when asked about.
- **Speech fallback never fell back** from `spd-say` to `espeak`, and `spd-say` didn't wait for speech to finish. `cargo clippy` passes again.
- **Integer-format microphones** (16/32-bit) were rejected as unsupported.
- Context logged every 2 s, including clipboard text — now logged on change, as a length.
- Kokoro's `repo_id` and torch warnings on every load.
- **MAVIS stopping mid-session** — three places sliced strings by byte index and panicked on non-ASCII transcriptions or clipboard text; the panic silently ended that subsystem. Most likely cause of the intermittent "doesn't respond" reports.
- **Event bus poison cascade** — one panic while holding its lock made every later publish panic.
- **Audio-thread deadlock** after any mutex poisoning.
- **GNOME session crash** — project detection rescanned `/proc` tens of thousands of times every 2 seconds.
- **Compositor misdetection** from leaked environment variables; failed commands being read as success.
- **Voice activity detection** — silence was undetectable because the threshold sat below the noise floor; rebuilt from measured traces with start/end hysteresis.
- **Invented transcriptions** — from the audio stream's startup click, and from Whisper hallucinating on near-silence.
- **MAVIS hearing its own voice** — echo suppression checked a flag that nothing ever set.
- **MAVIS treating its own guesses as memories** — recall now returns only what the user said.
- **Conversation evicted by polling** — context updates no longer enter the working-memory ring.
- **"google how to mute a tab" muting the machine.**
- Missing microphone no longer aborts the whole process.
- App discovery now searches XDG data directories.
- Deny-list falsely refusing `rm -rf` on a project directory; `curl … | bash` slipping through.
- Audit log recording held actions as "blocked".
- Sentinel losing log entries written in the same second as its watermark.

### Removed
- App names in the code: special handling for particular editors, terminals and browsers, and a fixed list of app names in the speech recogniser's vocabulary. Both now come from what's installed and what windows report.
- `episodic.db` — written on every turn, never read. An existing `memory/episodic.db` can be deleted.
- Unused platform traits for audio and screen capture.
- `PermanentStore` and `SessionStore` — Phase 1 stubs nothing read.
- Duplicate Python VAD and microphone code; dead scaffold modules.
- Browser-extension approach to tab awareness (the receiving socket is kept).

### Known issues
See [`DECISIONS.md` §14](DECISIONS.md#14-open-issues). Most notably: nothing since Phase 6.5 has been tried on hardware — actions, confirmation, push to talk, the Sentinel — and `Cargo.lock` is not committed.

---

## [0.3.0] — 2026-08-12 · `v0.3.0-ai-worker`

### Added
- Rust ↔ Python bridge over a Unix domain socket with length-prefixed JSON.
- Local LLM via llama-cpp-python (Phi-3-mini, Q4_K_M) with CUDA offload.
- Worker lifecycle: eager spawn, health checks, idle model unload, crash recovery with auto-respawn.
- Prompt system with a defined MAVIS personality and working-memory injection.
- `MAVIS_PYTHON_PATH` for the worker's interpreter.

## [0.2.0] — 2026-08-06 · *Phase 2, not tagged*

### Added
- Context engine routing every event type, with working memory.
- SQLite-backed memory layers.
- Planner and executor, with `shell`, `app`, `notify`, `say` and `system` actions.
- System integration: DBus media, volume and brightness; hotkey socket; file watcher.
- Orb state machine.

## [0.1.0] — 2026-08-04 · *Phase 1, not tagged*

### Added
- Project scaffold with Rust runtime and Python worker.
- Async event bus (tokio broadcast).
- Subsystem stubs for all Phase 1 modules.