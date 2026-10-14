# Ikura Rust prototype

This is a macOS SDL2 prototype for 《家族計画～追憶～》. The existing Python
`inspect_isf.py` handles ISF instruction framing and exports JSON; Rust owns
the script VM, graphics surfaces, SDL2 window and mouse input.

The development boot now starts at the real `START:0x68` entry. It executes
the 14 `VSET` declarations, registers 100 Shift-JIS character names with
`CNS`, and stores `WP`, `WS`, and `SETFONTCOLOR` text-window settings. Setup
opcodes that are no-ops in the reference decoder are accepted explicitly.
`GC` clears the screen, `GN` preserves the reference decoder's no-op behavior,
and `AVIP` plays `DOLOGO.MPG` at 640×480. The next
`JP` continues to `START:0x9DB`. It executes `START → BGMLOAD → TCG → TITLE`, composes
the 640×480 title screen through `GL`, `GP`, `GO`, and `GC`, and pauses at
`TITLE:0x7B9` for input. The four menu hit areas come from `TITLE`'s `IH`
instructions. Clicking Start passes the selected index to `IG` and follows
the script branch into `SNR00`. The title image composed from this
full startup path matches the previous partial boot byte for byte.

FFmpeg decodes the movie; SDL displays its frames and plays its audio inside
the same game window. Escape during the movie skips directly to title loading.
The visible title is rendered from the VM's screen surface. Start now runs
through the first scene background and `PM` text pages. Japanese text is
decoded from Shift-JIS and composed into the original message-box graphic
(`KA_M.GG2`) at the window origin from `START`'s setup; the speaker line from a
`PM` speaker command uses the `SETFONTCOLOR` name palette, and pages advance on
mouse click. Scripted choices render from their `CSET` rectangles with a
hover highlight and select the branch with a click. Timed `GO` fades and `GP` transition commands cross-fade over
their scripted duration instead of landing instantly. Music requests resolve to
the extracted OGG files: `ML` starts looping playback through SDL_mixer, `MS`
stops it, and `SER` plays sound effects. Recorded voice plays from the bank
selected by `CNF` (`VOICE1`/`VOICE2`), and `MP` retains the reference engine's
no-op behavior. Later script instructions are still pending; Continue and
Extras may stop at an unsupported opcode. Exit closes the window.

## Run

Requirements: Rust, SDL2 and SDL2_mixer with pkg-config, Python 3, FFmpeg and FFprobe,
extracted game data, and the original `DOLOGO.MPG` video. On macOS,
`brew install ffmpeg sdl2_mixer` provides the media dependencies. By default the program looks for
the video in the extracted data directory, its parent, and the parent of the
Rust project directory. Set `IKURA_VIDEO_DIR` to the original game directory
if the file is elsewhere.

```sh
cd '/Users/a1/Desktop/极速下载好的文件/家族计画追忆/家族计画追忆/ikura-rust-prototype'
python3 tools/export_isf.py \
  --parser /Users/a1/Desktop/dev/ikura/game_data/inspect_isf.py \
  --decoded /Users/a1/Desktop/dev/ikura/game_data/ISF-decoded \
  --output data/scripts.json
cargo run -- /Users/a1/Desktop/dev/ikura/game_data
```

Move the mouse over the four choices and click. Escape closes the window.
For a headless check that the Start branch reaches `SNR00`:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-start
```

The final line should be `VM after Start: Some(("SNR00", 13288))` for this
game's extracted data. To run through the first three text pages headlessly:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-story
```

Both probes skip video playback. For a normal GUI run,
`IKURA_SKIP_VIDEO=1` also skips the movie. `IKURA_DUMP_TITLE=/tmp/title.png` saves the title
surface during the probe. `IKURA_DUMP_STORY=/tmp/story.png` saves the first
composed story page during `--probe-story`; `IKURA_PROBE_PAGES` and
`IKURA_PROBE_CHOICE` control how far the headless story walk goes and which
choice it takes; `IKURA_DUMP_CHOICE=/tmp/choice.png` saves the composed choice
overlay. `IKURA_FONT` overrides the font
file for Japanese text. `IKURA_TRACE_VM=1` prints each instruction position.

A long walk stops when the script waits on state the VM cannot produce on its
own (a frame counter or a timer owned by the window). That is reported as
`Story probe stalled at ...` with the exact position, and the GUI shows
`Script awaiting input` instead of dropping the script.

To compose a single page with a speaker header without walking the script:

```sh
cargo run --example story_probe -- /Users/a1/Desktop/dev/ikura/game_data /tmp/page.png
```

## Next work

The Python parser remains the source of bytecode decoding; the Rust VM handles
instruction behavior. Remaining engine work: the original choice-window
graphics (the current overlay follows the scripted rectangles but draws its own
frame), save and load UI (`F5`/`F7` already capture their settings), the
opcode payloads that are still guesses, and the window frame counters that
scripts poll while waiting for input (the runner reports those as a stall
instead of guessing).
