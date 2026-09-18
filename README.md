# skob

A skob is a soft thing: a ring of point masses held together by springs, with
spokes to its middle to keep it plump.  Verlet integrated -- a skob has no
stored velocity, only the gap between where it is and where it just was.

It lives in your terminal, drawn in braille, and if you ask it to it will live
in your shell instead: a real bash prompt with scrollback, where the text on the
bottom rows is solid ground and the skobs land on the letters.

```
cargo build --release
./target/release/skob                       # one skob
./target/release/skob --shell -rn 6         # six of them, random colours, in a shell
./target/release/skob --command "summon water 1200" --command "summon skob 2"
```

`skob -h` prints the whole of the help.

## The things

| thing        | what it is                                                 |
|--------------|------------------------------------------------------------|
| `skob`       | a soft ball, the original                                  |
| `gorb`       | a rigid orange sphere with a highlight, no two alike       |
| `box`        | four corners, no give at all, and it sinks                 |
| `amoeba`     | so slack it barely keeps its shape                         |
| `string`     | an open chain, a rope with no inside                       |
| `sand`       | one grain, one braille dot, and it piles the way sand does |
| `bigsand`    | a coarser grain, one cell, written `#`                     |
| `water`      | a grain that finds its own level, and can be swum in       |
| `bigwater`   | a coarser drop, one cell, written `~`                      |
| `balloon`    | small, vibrant, on a string; three of them lift a skob     |
| `bigballoon` | perfectly round, and takes a skob away on its own          |

`:summon <thing> [n] [size]` drops them in at random, that many rows tall.
`:place <thing> [n] [size]` then puts them wherever you click, until `:stop`,
and `:erase [r]` is the same brush backwards -- click or drag to rub things out,
r columns across.  `:clear` takes away all of it at once.

Put a balloon down on top of something and it is tied to it: how much it lifts
depends on how big the balloon is against how big the thing is, so one balloon
makes a skob bouncy, three take it away, and one big one does it alone.

Sand and water know about each other: water flows sideways until it is level,
sand sinks through it, skobs float in it, and boxes are too dense to bother and
go to the bottom.

Drag anything by any part of it with the mouse.  `;` or `:` opens the command
line, vim style; space pauses, `g` kills gravity, `s` and `h` soften and harden,
`r` starts over, `q` leaves.

## The shell

`--shell` turns the screen into a bash shell that scrolls the way a terminal
does.  In the bottom rows (`-s`, six by default) the text itself is the ground:
things land on the letters and fall through the gaps between the words.  Above
that they are ghosts, and any text they are standing on is redrawn in their own
colour.  Sand is solid to a skob, a whole cell at a time.

Jobs run in the background, so everything keeps squishing while they work, and
a job that talks faster than the screen can listen -- `tree /` -- has its output
clipped to the tail rather than being allowed to stall the frame.  `skob ...` at
the prompt runs the commands above; the flags work there too.

## Flags

```
-r, --random         start with a random colour
-n, --count <n>      how many skobs to spawn at the start
-c, --colour <n>     one 256-colour index for everything
-s, --solid <n>      how many shell rows things collide with (default 6)
-z, --size <n>       how many rows tall they are (default: a seventh)
-w, --white-eyes     white eyes, and darker skins for them to sit in
-u, --uniform        every one exactly --size, no variation
    --shell          live in a bash shell
    --command <cmd>  run a command at startup, as often as you like
    --frames <n>     run n frames and quit, for recording
```

## Layout

| file        | what is in it                                              |
|-------------|------------------------------------------------------------|
| `term.rs`   | raw mode, terminal size, and the frame buffer              |
| `kinds.rs`  | the bestiary: what each thing is made of                   |
| `world.rs`  | the physics: verlet, springs, letters, and the sandbox     |
| `draw.rs`   | field pixels to braille cells                              |
| `shell.rs`  | the embedded shell: editing, completion, history, jobs     |
| `app.rs`    | the state, and the little language you talk to it with     |
| `main.rs`   | flags, the frame loop, and what a keystroke means          |
