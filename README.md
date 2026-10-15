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
`PM` speaker command uses the same primary text color as the body, and pages advance on
mouse click. Composed text is never written back into the VM's graphics
surfaces: the message box is translucent, so baking it in made the next page
draw over the previous page's text and show through as ghosting. Scripted choices render from their `CSET` rectangles with a
hover highlight and select the branch with a click or keyboard. `IHK` and
`IHKDEF` configure directional navigation and the default selection for title
hit areas and choices. Timed `GO` fades and `GP`
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
Continue opens the save-slot load menu. Extras enters the original CG character
selector; some deeper gallery branches may still reach unsupported script
operations. Exit closes the window.

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

### 中文模式

中文模式读取本机已有的 `isf_sc.bin` 汉化补丁，生成仅供本机使用的文本包；
仓库不会包含补丁或生成的译文。先安装 Python 的 `pycryptodome`，再运行：

```sh
python3 -m pip install pycryptodome
python3 tools/export_chinese_patch.py \
  --patch ../isf_sc.bin \
  --scripts data/scripts.json \
  --decoded /Users/a1/Desktop/dev/ikura/game_data/ISF-decoded \
  --output data/zh-CN.json
IKURA_LANG=zh-CN cargo run -- /Users/a1/Desktop/dev/ikura/game_data
```

也可以设置 `IKURA_TRANSLATIONS` 指向其他位置的文本包。中文模式优先加载
`<game_data>/PNG-images/ggd_sc` 中已有的汉化图片，并默认使用独立的
`<game_data>/rust-saves-zh` 存档目录；`IKURA_SAVE_DIR` 仍可指定目录。
`IKURA_FONT` 可覆盖自动选择的中文字体。未启用 `IKURA_LANG` 时仍使用原版日文。

目前对这份原版脚本和现有补丁进行上下文匹配，63,728 条台词片段中有
60,856 条找到译文（约 95.5%）；选项 162/172、章节名 161/169、角色名
67/100 条匹配。未匹配片段主要是重复的省略号、短句和调试提示，仍显示原文；
部分界面文字也来自原版图片。需要传入 `--decoded` 才能读取补丁用于
区分同文异译的脚本上下文；省略它时只能匹配约 39% 的台词。
可用下面的命令检查开场的三个中文页面：

```sh
IKURA_LANG=zh-CN cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-story
```

Move the mouse over the four choices and click. Escape closes the window.
Story controls:

| Control | Action |
| --- | --- |
| Left click / Space / Return | Complete revealing text; press again to advance dialogue |
| Arrow keys / Return | Navigate and activate the current title item or scripted choice using `IHK` mappings |
| Hold Ctrl | Fast-forward dialogue and skip graphical transitions; stops at choices |
| A | Toggle automatic reading; waits for voice and reading time, stops at choices |
| H / right click | Hide or show the message window; clicking while hidden shows it without advancing |
| B | Open history; wheel/scrollbar/arrow keys scroll; click a voiced line to replay; B/Esc/right click closes |
| F10 / Settings button | Open settings: music/sound/voice volumes, text delay, auto wait, autosave, fullscreen |
| F5 | Quick save at a script-permitted dialogue or choice input boundary |
| F6 | Open the manual save menu |
| F8 / title menu Continue | Open the load menu |
| F9 | Restore the quick save directly |

The menu has 12 manual slots across two pages plus separate Quick and Auto buttons.
Cards show the script's chapter name, timestamp (UTC, marked `Z`), dialogue
excerpt, and a thumbnail of the scene with dialogue or choices. Use mouse
clicks or arrow keys + Enter; PgUp/PgDn or the page buttons change pages.
Tab switches Save/Load. Escape cancels confirmation or closes the menu.
Overwriting an occupied slot and loading from the menu during play require
confirmation. F5/F9 remain direct quick actions. Auto reading stops on opening
the menu, while script frames and text-reveal timers pause until it closes.
Delete/Backspace or the Delete button requests deletion of the selected manual
slot; right-click a card, Quick, or Auto to delete that slot. Every deletion
requires confirmation and removes recovery backups and legacy quicksave files
as well. F7 opens Auto and Q opens Quick within the save menu. The Auto slot is
load-only; manual saves cannot overwrite it.

Script waits (`F1` duration + `F2 / WRF`) process SDL events rather than
blocking the window. Left click, Space, Return, or held Ctrl skips the wait;
the skip event is consumed so it cannot also advance the following dialogue.
Escape or closing the window exits promptly. Other UI commands are deferred
until the runner returns to an input boundary. Zero-length waits do not drain
input. Timer polling yields for a real 16 ms interval in the GUI; headless
probes keep their virtual clock and do not sleep.

When a script switches back to `TITLE` and reaches its menu input, the UI
refreshes the title texture, clears story text and choice overlays, exits auto
reading, and resumes title-menu input. Voice and effects stop while the title
script's music remains active. This path was exercised with an isolated
`LS TITLE` fixture; it does not establish that every ending reaches it correctly.
An empty VM script stack now exits the GUI instead of leaving a frozen scene.

Saves live in `<game_data>/rust-saves/`: `quicksave.iks` and
`slot-01.iks` through `slot-12.iks`, plus the separate `autosave.iks`. Set `IKURA_SAVE_DIR` to use another
directory. The new container starts with `IKRSAVE2`, a small metadata/thumbnail
index, then a gzip-compressed JSON state. Listing slots only reads their small
indexes. The previous JSON `quicksave.json` is still read when no modern quick
save exists, and it is retained when a modern quick save is created. These are
prototype saves, not the original Windows engine's save format.

A save includes script/call/return
stacks, variables, flags, timers, dialogue and choice state, and PNG snapshots
of every graphics surface. Files use unique temporary names, are flushed and
synced, and replace the destination by rename. Overwriting a valid save retains
its previous contents in an adjacent `.bak`; corrupt primaries do not replace
existing backups. Load tries the backup when the primary is unreadable and
shows an explicit recovery notification. Empty, damaged, and incompatible slots
are visible in the menu; failures leave the scene in place and show a message.
Load
validates the format, script-export fingerprint, saved instruction addresses,
and graphics before changing the live state. Bad or incompatible saves leave
the running scene intact. Audio paths inside the game data directory are stored
relatively; legacy absolute OGG-audio paths can be resolved against the current
data directory after moving an installation. New saves retain the dialogue
history (including relative voice paths) and music/voice playback time.
Loading seeks the OGG music and decodes the remaining voice from its saved
millisecond offset through FFmpeg. These times are measured from playback start,
so audio-device buffering can introduce a small offset; sample-exact restoration
is not guaranteed. Old saves without playback times start audio at the beginning.
The custom slot menu provides save/load functionality.

Automatic saves use an independent slot: the first saveable dialogue boundary,
each new choice or chapter, and changed dialogue boundaries at least 30 seconds
apart. The same boundary is never saved repeatedly, and script save restrictions
are respected. Load it using Auto in the load menu. Automatic saves can be
turned off in Settings; a write failure shows a notification and is throttled.

Settings apply immediately and are written to `<save directory>/settings.json`
when the panel closes. They survive restarting the game. Three independent
volume sliders cover music, effects, and dialogue/history voices. Text delay
scales the script's PF value (0 percent reveals instantly; a scripted PF 0 stays
instant). Auto wait adds the chosen minimum delay to reading time and still
waits for voice completion. Settings also control autosaving and fullscreen.
Opening settings pauses dialogue reveal/auto timers and the current voice;
closing it resumes them. Reset defaults applies the initial configuration.
`F7` chapter names are decoded from Shift-JIS, stored with the VM, restored on
load, and printed with the save/load confirmation. Older saves without a
chapter name remain readable.

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
`F8 / SETFONTSTYLE` is applied to both the speaker and body: this game's style
`04` draws a black shadow offset by (2, 2), and `05` adds bold. macOS prefers
the regular Hiragino W3 face as the base; fontdue bold is synthesized with a
one-pixel horizontal stroke while keeping character advances fixed. Switching
styles clears the previous effect. The rasterized glyph edges may differ from
the original Windows renderer. Choices keep their own plain UI style.

### Mouse cursors

`IM` (`80`) registers a named cursor resource and `IC` (`81`) selects its ID.
The original assets are read directly from the SM2MPX `DATA` cabinet:

| ID | Resource | Frames | Hotspot |
| --- | --- | --- | --- |
| 0 | `KA_TEXT` | 5 | (15, 15) |
| 3 | `KA_CUR` | 1 | (0, 0) |
| 4 | `KA_AUTO` | 8 | (15, 15) |
| 5 | `KA_BACK` | 5 | (15, 15) |

CUR decoding preserves palette colors, the transparency mask, bottom-up rows,
and cursor hotspots. ANI playback respects the frame sequence and individual
rates in 1/60-second units; this game's unusual RIFF length fields are handled
explicitly. Cursors are native SDL cursors, updated during dialogue, effects,
and script waits. Auto reading temporarily selects ID 4; leaving auto mode
returns to the script's selected ID. ID 5 is registered and usable by `IC`.
Pressing B during dialogue opens a history panel over the current scene.
Up/Down scroll one wrapped line; PgUp/PgDn scroll eight; Home/End jump to the
oldest/latest lines. The mouse wheel scrolls three lines per step and the right
scrollbar can be dragged. Click a voiced line (marked >) to replay its page's
voice clips in sequence. Replay uses a separate channel, pauses the current
story voice, and never advances the VM. B/Esc, right click, or Back closes the
panel, stops replay, and resumes the story voice and text timers. It keeps up to
2000 pages, clears for a new game, and is restored from new saves. Old saves
cannot recover history that was never stored. The interface uses the custom
port design.

Set `IKURA_CURSOR_DATA=/path/to/original/DATA` to choose the cabinet. Otherwise
the application checks the game data directory, its parent, and the original
game directory above this Rust project. Without the cabinet it logs a warning
and uses the system arrow. No cursor images are bundled in the repository.
New saves retain IM registrations and the selected ID; older saves retain the
startup registrations when restored. `IMS`/`IXY` mouse confinement and position
commands are implemented as SDL mouse confinement and warping. Those opcodes
do not occur in this game's exported scripts, so `IMS` currently
assumes left/top/right/bottom coordinates, matching the related `IH` rectangle
command; the game's decoded scripts cannot verify that payload convention.

The file layouts follow Microsoft's
[cursor resource format documentation](https://learn.microsoft.com/en-us/windows/win32/menurc/resource-file-formats).
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
final page count, choice count, visited scripts, stop reason, and VM position.
`IKURA_PROBE_CHOICE` selects a repeated choice (default 0); set
`IKURA_PROBE_CHOICES=0,1,0,...` to provide a per-choice route prefix, after
which the repeated choice is used. Unsupported operations and unresolved stalls
exit with a nonzero status; reaching the requested page limit and finishing
the script are reported separately.

To preview the animation renderer in SDL without walking the story:

```sh
cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --demo-animation
```

The demo plays fade, both blind directions, wipe, shake, and scrolling, then
exits. `IKURA_DUMP_ANIMATION=/tmp/animation.png` saves three intermediate frames
of each effect in a contact sheet. Shake, blinds, scrolling, and final-frame
restoration have deterministic pixel tests. `GSCRL` is tested with a synthetic
payload because this game's exported scripts do not contain that opcode.
`GGE` reads its named `EFF*.BIN`/`SEF*.BIN` resource from the original DATA
cabinet. These files are 320x240 BMP grayscale maps. The current renderer sweeps
a threshold through the original mask for the observed kind 0 / parameter 2
combination and supports both direction values. The reveal threshold, direction
mapping, and timing still need comparison against the original game; other GGE
combinations and missing DATA use a logged cross-fade fallback.

`GP 15/16/17` now follows the public IkuraDroid renderer: horizontal and vertical
slats use a 32-pixel pitch, the single-slat wipe uses the full width, and the
shrinking slats are painted with the script's packed 6-bit RGB color. The source
documents its implementation in
[ikuradecoder.cpp](https://github.com/VienDesuPorting/ikuradroid/blob/main/app/src/main/jni/vile/ikura/ikuradecoder.cpp)
and [blinds.cpp](https://github.com/VienDesuPorting/ikuradroid/blob/main/app/src/main/jni/vile/widgets/blinds.cpp).
`GP 24/25/28/29` and shake timing still need original-engine behavior and
reference-frame comparison. The public IkuraDroid renderer does not implement
those GP commands, and its GGE handler explicitly falls back to a fade, so its
source cannot establish those exact results. Set `IKURA_DATA=/path/to/DATA` to
select the cabinet for effects and cursors (`IKURA_CURSOR_DATA` still overrides
cursors alone). Corrupt or missing named entries in an existing cabinet report
an error.

To play all seven referenced masks in both directions:

```sh
IKURA_DUMP_MASKS=/tmp/ikura-mask-preview.png cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --demo-masks
```

The PNG contains one row per map (`EFF01`, `EFF03`, `EFF04`, `EFF17`, `SEF03`,
`SEF04`, `SEF05`); columns 1–3 show the normal sweep at 25/50/75 percent, and
columns 4–6 show the reverse sweep. The demo uses the same SDL transition player
as the script runner. Deterministic tests cover direction, scaling, initial and
final frames, bytecode parameters, and decoding all seven actual resources.
`GP 15/16/17` uses the public IkuraDroid slat renderer and parses its optional
packed color fields, but timing and shake motion have not been compared with the
original engine. `GP 24/25/28/29` still uses a logged cross-fade approximation;
the payload fields are decoded for analysis, but their visual semantics remain
unknown. The separate `GA*` animation-resource commands do not occur in this
game's scripts.

Scene scripts poll an engine-owned frame counter (script register `0x1F`) in
tight cycles while an animation or a text reveal plays. The VM models that
counter: `Vm::tick_frame` advances it once per rendered frame, the window calls
it every frame, and both the boot path and the GUI hand out frames until the
script reaches a boundary. A long walk still stops if a cycle needs state the
VM cannot produce; it is reported as `Story probe stalled at ...` with the exact
position. `IKURA_PROBE_FRAMES` sets how many additional frame retries the
headless walk may use when a long timer reaches the per-call instruction
budget. This game's ending credits include a roughly 90-second script timer.

To compose a single page with a speaker header without walking the script:

```sh
cargo run --example story_probe -- /Users/a1/Desktop/dev/ikura/game_data /tmp/page.png
```

## Next work

The Python parser remains the source of bytecode decoding; the Rust VM handles
instruction behavior. Choices, multi-slot saves, settings, and history use the
custom port interface. Remaining engine work includes opcode payloads whose
semantics are still inferred and exact reproduction of the special graphics
effects. A long walk now passes the previously blocking `MPM`
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
`IKURA_PROBE_FONT_STYLE=4` or `5` previews shadow and bold-plus-shadow text.

Latest route completion: an alternating `0,1` choice prefix with fallback 0
reached the ending sequence and returned to `TITLE` after 30,296 pages and 83
choices. Constant choice 1 also returned to `TITLE` after 30,278 pages and 82
choices. Both paths visited `SNR00`–`SNR11`, `SNRBE`, `STAFF`, and `TITLE`.
These are two headless choice paths; they do not cover every combination or
verify real-time audio/video timing. During gameplay and credits, mode-1 `IG`
polls now yield to the event loop so the player can provide input.

To render the current settings/history/save/delete panels with sample content:

```sh
cargo run --example interface_probe -- /tmp/ikura-interface-probe
```

For an SDL audio integration check (dummy audio avoids playing sound):

```sh
SDL_AUDIODRIVER=dummy cargo run -- /Users/a1/Desktop/dev/ikura/game_data data/scripts.json --probe-audio-ui
```

The check verifies independent volumes, history replay on a separate channel,
pausing/resuming the story voice, and seeking saved music/voice positions.
