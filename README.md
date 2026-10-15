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
The window title stays ASCII because SDL's macOS backend mangles non-ASCII
titles; the game's own name is set as the application name instead. The visible
title is rendered from the VM's screen surface. Start now runs
through the first scene background and `PM` text pages. Japanese text is
decoded from Shift-JIS and composed into the original message-box graphic
(`KA_M.GG2`) at the window origin from `START`'s setup; the speaker line from a
`PM` speaker command uses the `SETFONTCOLOR` name palette, and pages advance on
mouse click. Composed text is never written back into the VM's graphics
surfaces: the message box is translucent, so baking it in made the next page
draw over the previous page's text and show through as ghosting. Scripted choices render from their `CSET` rectangles with a
hover highlight and select the branch with a click. Timed `GO` fades and `GP`
transitions run in sequence over their scripted durations. `GP` commands 7/19
cross-fade, 15/16 reveal horizontal/vertical blinds, and 17 wipes left to right.
`GV` (`0x63`) shakes the display and restores its original position; `GSCRL`
(`0x62`) pans a viewport across a source surface and keeps the final viewport.
Animation playback pumps SDL events, so Escape/click skips the current effect
and closing the window interrupts playback. Music requests resolve to
the extracted OGG files: `ML` starts looping playback through SDL_mixer, `MS`
stops it, and `SER` plays sound effects. Recorded voice plays from the bank
selected by `CNF` (`VOICE1`/`VOICE2`), and `MP` retains the reference engine's
no-op behavior. Advancing dialogue (click, Space/Return, or Ctrl skip) and
selecting a choice stop the previous voice before resuming the script. Merely
showing a hidden dialogue box leaves playback running. Background music keeps
playing until the script stops, fades, or replaces it.
Selecting New Game explicitly stops the title track before running the story;
the title script clears its music registers without a standalone MS/MF there.
Faded music is excluded from quicksaves, and loading preserves the VM's music
stop/fade state. Legacy saves use this game's `BGMLOAD` track register to avoid
restarting a stale track after a scripted fade. A save taken during a fade
restores silence rather than restarting the fading song.
Later script instructions are still pending; Continue and
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
Story controls:

| Control | Action |
| --- | --- |
| Left click / Space / Return | Complete revealing text; press again to advance dialogue |
| Hold Ctrl | Fast-forward dialogue and skip graphical transitions; stops at choices |
| A | Toggle automatic reading; waits for voice and reading time, stops at choices |
| H / right click | Hide or show the message window; clicking while hidden shows it without advancing |
| F5 | Quick save at a script-permitted dialogue or choice input boundary |
| F9 / title menu Continue | Restore the quick save |

Quick saves are stored at `<game_data>/rust-saves/quicksave.json`. Set
`IKURA_SAVE_DIR` to use another directory. A save includes script/call/return
stacks, variables, flags, timers, dialogue and choice state, and PNG snapshots
of every graphics surface. Files are replaced through a temporary file. Load
validates the format, script-export fingerprint, saved instruction addresses,
and graphics before changing the live state. Bad or incompatible saves leave
the running scene intact. Background music and an active voice clip restart
from their beginnings after load; exact audio sample positions are not saved.
This is one quick slot, not the original engine's multi-slot save dialog.

The script's `WO`/`WC` visibility is applied when drawing dialogue.
`PM` and `MPM` pages now reveal their body text over time; speaker names appear
immediately. `PF` controls milliseconds per character (`0` is instant), with a
25 ms initial interval matching the reference engine. Layout is calculated from
the full page so reveal does not change font size or wrapping. Completing text
with a click preserves the current voice; advancing on the next click stops it.
The continue marker blinks every 500 ms after the text completes. Ctrl reveals
the full page immediately. Auto reading allows time for the reveal as well as
the reading delay and voice. Quicksaves store the whole page; loading shows it
fully revealed rather than replaying its animation. This implements progressive
text display, not lip synchronization or per-glyph fade/slide effects.
`PCMGETPOS` now updates the requested flag from the SDL voice channel, and
`SET` stops the current sound-effect channel. Auto reading uses the UI timer;
it does not implement the original engine's internal auto-speed settings.

For a headless check that the Start branch reaches `SNR00`:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-start
```

The final line should be `VM after Start: Some(("SNR00", 13288))` for this
game's extracted data. To run through the first three text pages headlessly:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-story
```

To save the first real story boundary, advance, and verify that restoring
returns the VM state and screen pixels exactly:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-save
```

This uses and removes a temporary save. `IKURA_PROBE_SAVE_OUTPUT` optionally
keeps the fixture at a specified path for manual UI testing.

Both probes skip video playback. For a normal GUI run,
`IKURA_SKIP_VIDEO=1` also skips the movie. `IKURA_DUMP_TITLE=/tmp/title.png` saves the title
surface during the probe. `IKURA_DUMP_STORY=/tmp/story.png` saves the first
composed story page during `--probe-story`; `IKURA_PROBE_PAGES` and
`IKURA_PROBE_CHOICE` control how far the headless story walk goes and which
choice it takes; `IKURA_DUMP_CHOICE=/tmp/choice.png` saves the composed choice
overlay. `IKURA_FONT` overrides the font
file for Japanese text. `IKURA_TRACE_VM=1` prints each instruction position.

`IKURA_PROBE_PAGES` accepts up to 100000 pages (default 3). The probe prints a
final page count, stop reason, and VM position. Unsupported operations and
unresolved stalls exit with a nonzero status; reaching the requested page limit
and finishing the script are reported separately.

To preview the animation renderer in SDL without walking the story:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --demo-animation
```

The demo plays fade, both blind directions, wipe, shake, and scrolling, then
exits. `IKURA_DUMP_ANIMATION=/tmp/animation.png` saves three intermediate frames
of each effect in a contact sheet. Shake, blinds, scrolling, and final-frame
restoration have deterministic pixel tests. `GSCRL` is tested with a synthetic
payload because this game's exported scripts do not contain that opcode.
`GGE` currently fades to the requested surface, following the reference
decoder's fallback; its external `SEF*.BIN` effect program is not interpreted.
Blind stripe spacing and shake motion are approximations rather than a verified
match to the original engine. `GP` 24/25/28/29 now preserve the requested source
image/rectangle and duration using a logged cross-fade approximation; their
extended effect parameters are not interpreted. The separate `GA*`
animation-resource commands do not occur in this game's scripts.

Scene scripts poll an engine-owned frame counter (script register `0x1F`) in
tight cycles while an animation or a text reveal plays. The VM models that
counter: `Vm::tick_frame` advances it once per rendered frame, the window calls
it every frame, and both the boot path and the GUI hand out frames until the
script reaches a boundary. A long walk still stops if a cycle needs state the
VM cannot produce; it is reported as `Story probe stalled at ...` with the exact
position. `IKURA_PROBE_FRAMES` sets how many frames the headless walk may spend
on such a cycle before reporting failure.

To compose a single page with a speaker header without walking the script:

```sh
cargo run --example story_probe -- /Users/a1/Desktop/dev/ikura/game_data /tmp/page.png
```

## Next work

The Python parser remains the source of bytecode decoding; the Rust VM handles
instruction behavior. Remaining engine work: the original choice-window
graphics (the current overlay follows the scripted rectangles but draws its own
frame), the original multi-slot save/settings dialogs, the
opcode payloads that are still guesses, and exact reproduction of the special
graphics effects. A long walk now passes the previously blocking `MPM`
simultaneous-dialogue page in `SNR00` and continues across chapter boundaries.
This verifies the walked paths, not the entire game or every choice combination.

Latest long-walk verification: both constant-choice strategies completed 10000
pages, covering `SNR00` through `SNR04`. Always choosing 0 exercised 29 choices
and 4 simultaneous-dialogue groups; always choosing 1 exercised 28 choices and
3 groups. The requested audio assets (3615 and 3613 distinct paths respectively)
all exist. This is headless execution and file-existence validation, not a
real-time audiovisual check of all those scenes.

The script's numeric operands use signed 30-bit immediates: `0x3FFFFFFF` means
`-1`, and `0x3FFFFFEC` means `-20`. Register operands use bit 31 as a reference
marker and retain their decoded signed arithmetic value. Correct sign extension
lets the choice-status comparison (`result > -1`) exit and also fixes negative
image coordinates. Regression tests cover both first-choice results.

Consecutive `PM` blocks append to the current page until an inline `03` clear;
the runner executes through them and pauses at the script's actual input poll.
This preserves multiline pages and prevents an early click from being discarded
by the following `IGINIT`.

`MPM` (`0xD6`) buffers numbered dialogue records and displays them in index
order when the script issues its group-display command. Each line keeps its
speaker name, and the records are cleared after display. This implements the
record/display payloads found in this game's scripts; it does not reproduce
the original engine's exact simultaneous text-reveal timing. Long group pages
reduce the font size when needed (down to 12px) to keep five speakers visible.
The composition probe accepts `IKURA_PROBE_SPEAKER=""` for a body-only group
page and `IKURA_PROBE_PAGE` for multiline text.
