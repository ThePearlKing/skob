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
| `amoeba`     | a bag of fluid: keeps its area, not its shape               |
| `string`     | an open chain, a rope with no inside                       |
| `sand`       | one grain, one braille dot, and it piles the way sand does |
| `bigsand`    | a coarser grain, one cell, written `#`                     |
| `water`      | a grain that finds its own level, and can be swum in       |
| `bigwater`   | a coarser drop, one cell, written `~`                      |
| `balloon`    | small, vibrant, on a string; three of them lift a skob     |
| `bigballoon` | perfectly round, and takes a skob away on its own          |

`:summon <thing> [n] [size]` drops them in at random, that many rows tall.
`:place <thing> [n] [size]` then draws them with the mouse -- hold the button
down and sweep, and nothing is put down inside anything already there, so you
can lay a wall out a block at a time -- until `:stop`,
and `:erase [r]` is the same brush backwards -- click or drag to rub things out,
r columns across.  `:clear` takes away all of it at once.

Put a balloon down on top of something and it is tied to it: how much it lifts
depends on how big the balloon is against how big the thing is, so one balloon
makes a skob bouncy, three take it away, and one big one does it alone.

Sand and water know about each other: water flows sideways until it is level,
sand sinks through it, skobs float in it, and boxes are too dense to bother and
go to the bottom.

An amoeba is not a soft ball but a bag.  It holds the area inside it rather
than any particular radius, and carries more skin than a circle of that area
needs, so there is no shape for it to spring back to and nothing anywhere
tells it what shape to be: what it looks like is only ever the sum of what has
happened to it.  It puddles where it lands, keeps the dent you drag into it,
and leans on its own skin from the inside, harder in some places than others,
which is how it puts out a pseudopod and oozes off across the floor.  Where it
leans wanders of its own accord and fades, and never repeats.  You can see the
membrane, the nucleus and a vacuole or two through it.

Anywhere skob takes a number it will take a spread instead, written with a
dash, and rolls it afresh every time it is used rather than once when it is
read:

```
:summon skob 1-4 5-13     one to four skobs, each its own size between five
                          and thirteen rows -- a roll for how many, then a
                          roll each for how big
:place gorb 1-3 2-6       and again at every click
:colour 20-200            every one of them a colour of its own
:gravity 0.05-0.4         one roll, because the world has one gravity
skob -n 2-9 -z 3-8        flags too
```

A size given as a spread is taken as read: the little wander in size that
things otherwise come with is what a spread replaces, not something added on
top of it.

Commands can be strung together with `;`:
`:clear ; summon amoeba 3 ; gravity 0.1` does all three, in that order.

Drag anything by any part of it with the mouse.  `;` or `:` opens the command
line, vim style; space pauses, `g` kills gravity, `s` and `h` soften and harden,
`r` starts over, `q` leaves.

## The shell

`--shell` turns the screen into a bash shell that scrolls the way a terminal
does.  Anything that draws its own screen -- an editor, a pager, `htop`,
`cmatrix`, or one of the fetch tools that prints beside its own logo -- is
handed the real terminal for as long as it runs, and the screen is held until
you press a key so you can read what it left.  Everything else runs as a job
whose output is read as text: colour is kept, and anything it says that would
have moved the cursor or taken the screen is dropped, because that would have
been done to the shell rather than by it.  In the bottom rows (`-s`, six by default) the text itself is the ground:
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
