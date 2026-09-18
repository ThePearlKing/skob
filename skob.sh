#!/usr/bin/env bash
# ===========================================================
#  s k o b
#
#  A skob is a soft thing: a ring of point masses held together
#  by springs, with spokes to its middle to keep it plump.
#  Verlet integrated -- a skob has no stored velocity, only the
#  gap between where it is and where it just was.
#
#  FLAGS
#    -r, --random         start with random colours
#    -n, --count <n>      how many skobs to spawn at the start
#        --skobs <n>      (the old spelling of --count)
#    -c, --colour <n>     start with this 256-colour index
#        --shell          live in a bash shell (see below)
#    -s, --solid <n>      how many shell rows he collides with (default 6)
#    -z, --size <n>       how many rows tall he is (default: a seventh)
#    -w, --white-eyes     white eyes, and darker skins for them to sit in
#    -u, --uniform        every skob exactly --size, no variation
#        --command <cmd>  run a command at startup; repeat it as often as you like
#                         (skob --command "summon sand 100" --command "place box")
#        --frames <n>     run n frames and quit (for recording)
#    -h, --help           this
#
#  MOUSE  press and drag -> grab a skob's skin and fling it
#  KEYS   space pause · g gravity · b bouncier · s softer
#         h harder · r reset · q quit
#  ;  or  :   open the command line, vim style
#
#  THINGS   skob    a soft ball, the original
#           gorb    an orange sphere, rigid, no two quite the same shade
#           box     four corners and no give at all
#           amoeba  so slack it barely keeps its shape
#           string  an open chain, a rope with no inside
#           sand    one grain, and it piles the way sand does
#           bigsand a coarser grain, written #
#
#  COMMANDS
#    :summon <thing> [n]  summon n of them, dropped in at random
#    :place  <thing> [n]  then every click puts n of them where you clicked,
#                         until :stop
#    :stop              stop placing
#    :clear             banish every skob
#    :reset             back to one skob
#    :gravity <x>       set gravity      (0 = weightless)
#    :stiffness <x>     set spring stiffness
#    :bounce <x>        set wall bounce
#    :colour <n>        recolour every skob (256-colour index)
#    :q  :quit          leave
#
#  --shell
#    The whole screen becomes a bash shell, scrolling upward the
#    way a terminal does.  In the bottom 6 lines (-s) the text
#    itself is the ground: he lands on the letters and falls
#    through the gaps between the words.  Everything above
#    is scrollback he drifts through -- and any text he is on top
#    of is redrawn in his own colour, so you read him and the
#    text at once.  His eyes stay dark and are never covered.
#    When new output arrives the buffer scrolls, and the rising
#    lines shove him up rather than swallowing him.
#
#    Inside the shell, `skob ...` runs the commands listed above
#    (skob summon 3, skob gravity 0, skob help).
#
#    SHELL KEYS  enter run · tab complete · up/down history
#                left/right & ctrl-arrow move · ctrl-a/e ends
#                ctrl-u/k/w kill · ctrl-l clear · ctrl-c abort
#                ctrl-d leave.  Long jobs run in the background,
#                so he keeps squishing while they work.
# ===========================================================
set -u
ESC=$'\033'
FRAMES=0; TICK_US=55000; PAUSED=0; QUIT=0
GRAV=0.32; STIFF=0.55; BOUNCE=0.45; FRIC=0.995
GRABS=-1; GRABI=-1; MX=-1; MY=-1; NOTE=""
PLACING=""; PLACEN=1; CMDS=()
MODE=normal; CMD=""
INIT=""; RANDCOL=0; SHELL_MODE=0; SOLID=6; BASECOL=84; SIZE=""; EYECOL=232; VARY=1; DARK=0
SPIN=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧)

usage(){ sed -n '2,69p' "$0" | sed 's/^# \{0,1\}//'; }

# a highlight is the colour of the thing with more light on it, never white
LITCACHE=()
lighten(){ local c="$1" r g b
  if [ -n "${LITCACHE[$c]:-}" ]; then LITOUT="${LITCACHE[$c]}"; return; fi
  if (( c>=16 && c<=231 )); then
    r=$(( (c-16)/36 )); g=$(( ((c-16)%36)/6 )); b=$(( (c-16)%6 ))
    r=$(( r+2>5?5:r+2 )); g=$(( g+2>5?5:g+2 )); b=$(( b+2>5?5:b+2 ))
    LITOUT=$(( 16 + r*36 + g*6 + b ))
  elif (( c>=232 && c<=249 )); then LITOUT=$(( c+6 ))
  elif (( c>=250 )); then LITOUT=255
  elif (( c<=7 )); then LITOUT=$(( c+8 ))
  else LITOUT="$c"; fi
  LITCACHE[$c]="$LITOUT"; }

# -w wants white eyes, so everything else has to get out of their way
dim_col(){ local c="$1" r g b
  (( DARK )) || { printf '%s' "$c"; return; }
  if (( c>=16 && c<=231 )); then
    r=$(( (c-16)/36 )); g=$(( ((c-16)%36)/6 )); b=$(( (c-16)%6 ))
    r=$(( (r+1)/2 )); g=$(( (g+1)/2 )); b=$(( (b+1)/2 ))
    printf '%s' $(( 16 + r*36 + g*6 + b ))
  elif (( c>=232 )); then
    c=$(( 232 + (c-232)/2 )); printf '%s' "$c"
  elif (( c>7 )); then printf '%s' $(( c-8 ))
  else printf '%s' "$c"; fi
}

# any of the 256 terminal colours, so long as it is actually visible
rand_colour(){ local c r g b
  while :; do
    c=$(( RANDOM % 216 + 16 ))                 # the 6x6x6 colour cube
    r=$(( (c-16)/36 )); g=$(( ((c-16)%36)/6 )); b=$(( (c-16)%6 ))
    (( r+g+b >= 5 )) && { dim_col "$c"; return; }   # skip the near-blacks
  done
}

while [ $# -gt 0 ]; do
  # a bundle of short flags, e.g. -rn 6 or -rn6, becomes -r -n 6
  case "$1" in -[!0-9-]?*)
    s="${1#-}"; shift; bundle=()
    while [ -n "$s" ]; do
      c="${s:0:1}"; s="${s:1}"
      case "$c" in
        n|c|s|z) bundle+=("-$c"); [ -n "$s" ] && { bundle+=("$s"); s=""; };;
        *)    bundle+=("-$c");;
      esac
    done
    set -- "${bundle[@]}" "$@"
  esac
  case "$1" in
  --frames)            FRAMES="$2"; shift 2;;
  -n|--count|--skobs)  INIT="$2"; shift 2;;
  -r|--random)         RANDCOL=1; shift;;
  -c|--colour|--color) BASECOL="$2"; shift 2;;
  --shell)             SHELL_MODE=1; shift;;
  --command|--cmd)     CMDS+=("$2"); shift 2;;
  -s|--solid)          SOLID="$2"; shift 2;;
  -z|--size)           SIZE="$2"; shift 2;;
  -w|--white-eyes)     EYECOL=231; DARK=1; shift;;
  -u|--uniform|--no-variation) VARY=0; shift;;
  -h|--help)           usage; exit 0;;
  -[0-9]*)             INIT="${1#-}"; shift;;
  *) shift;;
  esac
done
[ "$SOLID" -lt 2 ] 2>/dev/null && SOLID=2

now_us(){ local t=${EPOCHREALTIME/./}; printf '%s' "${t:-0}"; }
if (( SHELL_MODE )); then          # the shell keeps its cursor
  ON="${ESC}[?1049h${ESC}[?7l${ESC}[?1003h${ESC}[?1006h"
else
  ON="${ESC}[?1049h${ESC}[?7l${ESC}[?25l${ESC}[?1003h${ESC}[?1006h"
fi
OFF="${ESC}[?1003l${ESC}[?1006l${ESC}[?7h${ESC}[?25h${ESC}[?1049l"
STTY_SAVE=""; command -v stty >/dev/null && STTY_SAVE=$(stty -g 2>/dev/null || true)
RUNDIR="${TMPDIR:-/tmp}/skob.$$"; RUNOUT="$RUNDIR/out"; RUNMETA="$RUNDIR/meta"
cleanup(){ [ -n "${RUNPID:-}" ] && kill "$RUNPID" 2>/dev/null
  [ -n "$STTY_SAVE" ] && stty "$STTY_SAVE" 2>/dev/null
  rm -rf "$RUNDIR" 2>/dev/null
  printf '%s%s[0m' "$OFF" "$ESC"; exit 0; }
trap cleanup INT TERM EXIT

BR=(); for ((b=0;b<256;b++)); do BR[$b]=$(printf "\\u$(printf '%04x' $((0x2800+b)))"); done
DOT=(1 2 4 64 8 16 32 128)

COLS=$(tput cols); ROWS=$(tput lines)
playrows(){ echo $(( ROWS-1 )); }
PW=$(( COLS*2 )); PH=$(( $(playrows)*4 ))

# PTS entries: "sid lidx x y ox oy"      SK_* are indexed by sid
PTS=(); SK_N=(); SK_REST=(); SK_COL=(); SK_KIND=(); SK_PHYS=(); NEXTID=0
SK_MODE=(); SK_EYES=(); SK_SHINE=(); SK_GLYPH=()

# what each kind is made of, in order:
#   points phase stiffness shellmin shellmax twist mode size eyes glyph shine
#   mode  0 = a ring of skin round a middle · 1 = an open chain
#         2 = a grain one dot big · 3 = a grain one whole cell big
#   size  0 means "whatever --size says", anything else is fixed
#   glyph - means draw it in braille like everything else
#   only the living things get eyes
bestiary(){ case "$1" in
  skob)    SPEC="14 0.0000 1.00 0.38 1.65 0.85 0 0 1 - 0";;
  gorb)    SPEC="16 0.0000 1.00 0.94 1.06 0.20 0 0 0 - 1";;  # a sphere, and it shines
  box)     SPEC="4 0.7854 1.00 0.97 1.03 0.04 0 0 0 - 0";;   # four corners, no give
  amoeba)  SPEC="18 0.0000 0.30 0.12 2.40 1.40 0 0 1 - 0";;  # barely holds its shape
  string)  SPEC="16 0.0000 1.00 0.00 0.00 0.00 1 0 0 - 0";;  # a chain, no inside at all
  sand)    SPEC="1 0.0000 1.00 0.00 0.00 0.00 2 1 0 - 0";;   # a grain, and it piles
  bigsand) SPEC="1 0.0000 1.00 0.00 0.00 0.00 3 4 0 # 0";;   # a coarse grain, a #
  *)       SPEC="";;
esac; }

# one gorb, two gorbs, but never two sands
many(){ case "$1" in sand|bigsand) printf '%s' "$1";; *) printf '%ss' "$1";; esac; }

# the radius a new one of this kind gets
kind_r(){ bestiary "$1"; local _n _p _k _lo _hi _l _m rad
  read -r _n _p _k _lo _hi _l _m rad _ <<< "$SPEC"
  if [ "${rad:-0}" != 0 ]; then printf '%s' "$rad"; else skob_r 1; fi; }

# every kind but the skob wears its own colours
kind_colour(){ local -a p
  case "$1" in
    gorb)   p=(202 208 214 220 209 215 166 172 178 216 221 203);;   # reddish to yellowish
    box)    p=(236 238 240 242 244 246 248 250 252 254 239 245);;
    amoeba) p=(71 77 78 83 107 113 114 120 148 150 79 85);;
    sand|bigsand) p=(179 180 186 187 222 223 221 215 214 143);;
    *)      start_col; return;;
  esac
  dim_col "${p[$(( RANDOM % ${#p[@]} ))]}"; }

add_skob(){ # add_skob <cx> <cy> <r> [colour] [kind]
  local cx=$1 cy=$2 rr=$3 col="${4:-}" kind="${5:-skob}" sid=$NEXTID
  bestiary "$kind"; [ -z "$SPEC" ] && return 1
  local n ph kk lo hi lim mode rad eyes glyph shine
  read -r n ph kk lo hi lim mode rad eyes glyph shine <<< "$SPEC"
  [ -z "$col" ] && col="$(rand_colour)"
  # one awk lays out every point: a ring and its middle, or a hanging chain
  mapfile -t pts < <(awk -v sid="$sid" -v cx="$cx" -v cy="$cy" -v r="$rr" \
                         -v n="$n" -v ph="$ph" -v mode="$mode" 'BEGIN{
    PI=3.14159265
    if(mode>=2){ printf "%s 0 %.3f %.3f %.3f %.3f\n", sid,cx,cy,cx,cy }
    else if(mode==1){ seg=int(r/3); if(seg<2) seg=2
      for(i=0;i<n;i++){ px=cx-(n-1)*seg/2+i*seg
        printf "%s %d %.3f %.3f %.3f %.3f\n", sid,i,px,cy,px,cy } }
    else{ for(i=0;i<n;i++){ a=2*PI*i/n+ph; px=cx+r*cos(a); py=cy+r*sin(a)
        printf "%s %d %.3f %.3f %.3f %.3f\n", sid,i,px,py,px,py }
      printf "%s %d %.3f %.3f %.3f %.3f\n", sid,n,cx,cy,cx,cy } }')
  PTS+=("${pts[@]}")
  if [ "$mode" = 1 ]; then rr=$(( rr/3 )); (( rr<2 )) && rr=2; fi   # rest = one link
  SK_N[$sid]=$n; SK_REST[$sid]=$rr; SK_COL[$sid]=$col; SK_KIND[$sid]="$kind"
  SK_PHYS[$sid]="$kk:$lo:$hi:$lim:$mode:$shine"; SK_MODE[$sid]="$mode"
  SK_EYES[$sid]="$eyes"; SK_SHINE[$sid]="$shine"
  if [ "$glyph" = "-" ]; then SK_GLYPH[$sid]=""; else SK_GLYPH[$sid]="$glyph"; fi
  NEXTID=$((NEXTID+1))
}

place_at(){ # place_at <px> <py> <kind> <n>   -- a handful where you clicked
  local cx=$1 cy=$2 kind=$3 k=$4 i c jx jy
  bestiary "$kind"; [ -z "$SPEC" ] && return 1
  for (( i=0; i<k; i++ )); do
    c="$(kind_colour "$kind")"
    if (( k > 1 )); then jx=$(( cx + RANDOM % 13 - 6 )); jy=$(( cy + RANDOM % 13 - 6 ))
    else jx=$cx; jy=$cy; fi
    (( jx<1 )) && jx=1; (( jx>PW-2 )) && jx=$((PW-2))
    (( jy<1 )) && jy=1; (( jy>PH-2 )) && jy=$((PH-2))
    add_skob "$jx" "$jy" "$(kind_r "$kind")" "$c" "$kind"
  done
  NOTE="placed $k $([ "$k" -gt 1 ] && many "$kind" || printf '%s' "$kind")"; }

banish(){ PTS=(); SK_N=(); SK_REST=(); SK_COL=(); SK_KIND=(); SK_PHYS=(); NEXTID=0
  SK_MODE=(); SK_EYES=(); SK_SHINE=(); SK_GLYPH=(); }

# the colour a freshly started skob wears: random with -r, else the base
start_col(){ if (( RANDCOL )); then rand_colour; else dim_col "$BASECOL"; fi; }

# his radius in half-cells: --size is his height in text rows, else a seventh.
# no two skobs are quite the same size unless -u says they must be.
skob_r(){ local r j
  if [ -n "$SIZE" ] && [[ "$SIZE" =~ ^[0-9]+$ ]]; then r=$(( SIZE*2 )); else r=$(( PH/7 )); fi
  if (( VARY && ${1:-0} )); then
    j=$(( r/5 )); (( j<1 )) && j=1
    r=$(( r + RANDOM % (2*j+1) - j ))
  fi
  (( r<2 )) && r=2; printf '%s' "$r"; }

reset_all(){ banish
  add_skob $(( PW/2 )) $(( PH/3 )) "$(skob_r)" "$(start_col)" skob; NOTE="a new skob"; }

summon(){ local k="${1:-1}" kind="${2:-skob}" col="${3:-}" i c
  [[ "$k" =~ ^[0-9]+$ ]] || k=1
  bestiary "$kind" || true
  [ -z "$SPEC" ] && { NOTE="never heard of a $kind"; return 1; }
  local cap=60; [ "${SPEC%% *}" = 1 ] && cap=600      # grains come by the handful
  (( k > cap )) && k=cap
  for (( i=0; i<k; i++ )); do
    c="$col"; [ -z "$c" ] && c="$(kind_colour "$kind")"
    add_skob $(( RANDOM % (PW-40>1?PW-40:1) + 20 )) $(( RANDOM % (PH/3>1?PH/3:1) + 6 )) \
             "$(kind_r "$kind")" "$c" "$kind"
  done
  NOTE="summoned $k $([ "$k" -gt 1 ] && many "$kind" || printf '%s' "$kind")"; }

reset_all
if [ -n "$INIT" ]; then
  banish
  if (( RANDCOL )); then summon "$INIT" skob; else summon "$INIT" skob "$BASECOL"; fi
fi

run_cmd(){
  local line="$1" verb arg1 arg2
  read -r verb arg1 arg2 <<< "$line"
  case "$verb" in
    summon) local kd=skob ct=1
            if [[ "$arg1" =~ ^[0-9]+$ ]]; then ct="$arg1"
            elif [ -n "$arg1" ]; then kd="$arg1"; [[ "$arg2" =~ ^[0-9]+$ ]] && ct="$arg2"; fi
            summon "$ct" "$kd";;
    place)  local pk=skob pn=1
            if [[ "$arg1" =~ ^[0-9]+$ ]]; then pn="$arg1"
            elif [ -n "$arg1" ]; then pk="$arg1"; [[ "$arg2" =~ ^[0-9]+$ ]] && pn="$arg2"; fi
            bestiary "$pk"
            if [ -z "$SPEC" ]; then NOTE="never heard of a $pk"
            else PLACING="$pk"; PLACEN="$pn"; NOTE="click to place $pk"; fi;;
    stop)   if [ -n "$PLACING" ]; then NOTE="stopped placing $PLACING"; PLACING=""
            else NOTE="not placing anything"; fi;;
    clear)  banish; NOTE="all skobs banished";;
    reset)  reset_all;;
    gravity)   GRAV="${arg1:-0.32}"; NOTE="gravity $GRAV";;
    stiffness) STIFF="${arg1:-0.55}"; NOTE="stiffness $STIFF";;
    bounce)    BOUNCE="${arg1:-0.45}"; NOTE="bounce $BOUNCE";;
    colour|color) local i; for i in "${!SK_COL[@]}"; do SK_COL[$i]="${arg1:-$BASECOL}"; done; NOTE="recoloured";;
    random) local i; for i in "${!SK_COL[@]}"; do SK_COL[$i]="$(rand_colour)"; done; RANDCOL=1; NOTE="recoloured at random";;
    pause) PAUSED=$((1-PAUSED)); NOTE=$([ $PAUSED = 1 ] && echo held || echo squishing);;
    # the flags work here too, and -h prints the whole of the help where you
    # can actually read it: down the scrollback, not squeezed onto one line
    -h|--help|help)
      if (( SHELL_MODE )); then
        local -a UL=(); mapfile -t UL < <(usage); local l
        for l in "${UL[@]}"; do sb_note "  $l"; done; NOTE=""
      else
        NOTE="summon|place skob|gorb|box|amoeba|string|sand|bigsand [n] · stop · clear · reset · gravity x · stiffness x · bounce x · colour n · random · pause · quit"
      fi;;
    -n|--count|--skobs)  summon "${arg1:-1}" skob;;
    -r|--random)         run_cmd "random";;
    -c|--colour|--color) run_cmd "colour ${arg1:-$BASECOL}";;
    -z|--size)           SIZE="$arg1"; NOTE="size ${arg1:-default}";;
    -u|--uniform)        VARY=$(( 1-VARY )); NOTE=$([ $VARY = 1 ] && echo "sizes vary again" || echo "every one the same size");;
    -s|--solid)          SOLID="${arg1:-6}"; (( SOLID<2 )) && SOLID=2; NOTE="$SOLID solid rows";;
    -w|--white-eyes)     if [ "$EYECOL" = 231 ]; then EYECOL=232; DARK=0; NOTE="dark eyes"
                         else EYECOL=231; DARK=1; NOTE="white eyes"; fi;;
    q|quit) QUIT=1;;
    "") :;;
    *) NOTE="not a command: $verb";;
  esac
}

# ---------------------------------------------------------------- the shell
CWD="$PWD"; LASTRC=0; LINE=""; CURPOS=0
SB=(); SBC=(); HIST=(); HISTI=0; HSAVE=""; NEWLINES=0; PENDPUSH=0; TOPTEXT=$ROWS
RUNPID=""; RUNPOS=0; PARTIAL=""
PPLAIN=""; PCOL=""; HLIN=$'\001none'; HLOUT=""; STRIPPED=""
declare -A CMDKIND=()
FG_CMDS=" vi vim nvim nano emacs pico less more man top htop btop atop watch ssh sudo su doas python python3 ipython node irb psql mysql sqlite3 gdb lldb tmux screen fzf ranger mc nmtui alsamixer crontab visudo "

_strip(){ local s="$1" out=""
  while [[ "$s" == *$'\033'* ]]; do
    out+="${s%%$'\033'*}"; s="${s#*$'\033'}"
    if [[ "$s" == '['* ]]; then s="${s#\[}"
      while [ -n "$s" ]; do case "${s:0:1}" in [A-Za-z]) break;; esac; s="${s:1}"; done
      s="${s:1}"
    else s="${s:1}"; fi
  done
  STRIPPED="$out$s"; }

sb_push(){ # sb_push <plain> <coloured>
  local p="${1//$'\t'/        }"; local c="${2//$'\t'/        }"
  SB+=("$p"); SBC+=("$c"); NEWLINES=$((NEWLINES+1))
  if (( ${#SB[@]} > 2000 )); then SB=("${SB[@]:600}"); SBC=("${SBC[@]:600}"); fi; }

sb_out(){ # a line of program output: keep whatever colour it brought
  local l="$1"
  case "$l" in                      # bash blames us by name; say "bash" instead
    "$0: line "*) l="${l#"$0": line }"; l="bash: ${l#*: }";;
  esac
  if [[ "$l" == *$'\033'* ]]; then _strip "$l"; sb_push "$STRIPPED" "$l"
  else sb_push "$l" "$l"; fi; }

sb_note(){ sb_push "$1" "${ESC}[38;5;244m$1${ESC}[0m"; }

KW=" if then else elif fi for while until do done case esac function in select time coproc return "

hl_kind(){ # sets HLK for a command word.  the kind is cached, the colour is not,
  local w="$1" t                      # because a skob may be recoloured at any time
  t="${CMDKIND[$w]:-}"
  if [ -z "$t" ]; then
    case "$w" in
      skob) t=skob;;
      */*)  if [ -x "$CWD/$w" ] || [ -x "$w" ]; then t=file; else t=none; fi;;
      *)    t=$(type -t "$w" 2>/dev/null) || t=none; [ -z "$t" ] && t=none;;
    esac
    CMDKIND[$w]="$t"
  fi
  case "$t" in
    builtin|keyword) HLK="1;38;5;176";;
    function|alias)  HLK="1;38;5;117";;
    file|skob)       HLK="1;38;5;${SK_COL[0]:-$BASECOL}";;
    *)               HLK="38;5;203";;
  esac; }

hl_raw(){ local s="$1"; local n=${#s} i=0 j q out="" w cmdpos=1 pend="" ch two
  while (( i<n )); do
    ch="${s:i:1}"
    case "$ch" in
      ' '|$'\t') out+="$ch"; ((i++)); continue;;
      '#') out+="${ESC}[38;5;244m${s:i}"; i=$n; continue;;
      '|'|'&'|';'|'<'|'>'|'('|')')
        two="${s:i:2}"
        case "$two" in '&&'|'||'|'>>'|'<<'|';;'|'|&') w="$two";; *) w="$ch";; esac
        out+="${ESC}[1;38;5;214m${w}${ESC}[0m"; i=$((i+${#w})); cmdpos=1; pend=""; continue;;
      "'")
        j=$((i+1)); while (( j<n )) && [ "${s:j:1}" != "'" ]; do ((j++)); done
        (( j<n )) && ((j++))
        out+="${ESC}[38;5;150m${s:i:j-i}${ESC}[0m"; i=$j; cmdpos=0; continue;;
      '"')
        j=$((i+1))
        while (( j<n )); do
          [ "${s:j:1}" = '\' ] && { j=$((j+2)); continue; }
          [ "${s:j:1}" = '"' ] && break; ((j++))
        done
        (( j<n )) && ((j++))
        out+="${ESC}[38;5;150m${s:i:j-i}${ESC}[0m"; i=$j; cmdpos=0; continue;;
      '$')
        j=$((i+1)); q="${s:j:1}"
        if [ "$q" = "{" ]; then while (( j<n )) && [ "${s:j:1}" != "}" ]; do ((j++)); done; (( j<n )) && ((j++))
        elif [ "$q" = "(" ]; then while (( j<n )) && [ "${s:j:1}" != ")" ]; do ((j++)); done; (( j<n )) && ((j++))
        else
          while (( j<n )); do case "${s:j:1}" in [A-Za-z0-9_]) ((j++));; *) break;; esac; done
          (( j==i+1 )) && ((j++))
        fi
        out+="${ESC}[1;38;5;117m${s:i:j-i}${ESC}[0m"; i=$j; cmdpos=0; continue;;
    esac
    j=$i
    while (( j<n )); do
      case "${s:j:1}" in ' '|$'\t'|'|'|'&'|';'|'<'|'>'|'('|')'|"'"|'"'|'$') break;; esac; ((j++))
    done
    w="${s:i:j-i}"; i=$j
    if [ -z "$w" ]; then ((i++)); continue; fi
    case " $KW " in                                # a keyword anywhere is a keyword
      *" $w "*) out+="${ESC}[1;38;5;176m${w}${ESC}[0m"
        case "$w" in
          for|select|function) cmdpos=0; pend=var;;   # ...then a name, not a command
          case|in)             cmdpos=0; pend="";;
          *)                   cmdpos=1; pend="";;
        esac
        continue;;
    esac
    if [ "$pend" = var ]; then                     # the loop variable
      out+="${ESC}[1;38;5;117m${w}${ESC}[0m"; pend=""; cmdpos=0; continue
    fi
    if (( cmdpos )); then
      if [[ "$w" =~ ^[A-Za-z_][A-Za-z0-9_]*=(.*)$ ]]; then    # VAR=value keeps command position
        out+="${ESC}[38;5;117m${w%%=*}${ESC}[1;38;5;214m=${ESC}[0m${ESC}[38;5;253m${w#*=}${ESC}[0m"; continue
      fi
      hl_kind "$w"; out+="${ESC}[${HLK}m${w}${ESC}[0m"; cmdpos=0; continue
    fi
    case "$w" in
      -*)        out+="${ESC}[38;5;110m${w}${ESC}[0m";;
      *[!0-9.]*) out+="${ESC}[38;5;253m${w}${ESC}[0m";;
      *)         out+="${ESC}[38;5;215m${w}${ESC}[0m";;
    esac
  done
  HLOUT="$out${ESC}[0m"; }

hl(){ [ "$HLIN" = "$1" ] && return; HLIN="$1"; hl_raw "$1"; }

mk_prompt(){
  local d="$CWD" rc=""
  [[ "$d" == "$HOME" ]] && d="~"
  [[ "$d" == "$HOME"/* ]] && d="~${d#"$HOME"}"
  [ "$LASTRC" != 0 ] && rc=" ✗${LASTRC}"
  local c="${SK_COL[0]:-$BASECOL}"
  PPLAIN="$d$rc \$ "
  PCOL="${ESC}[1;38;5;110m${d}${ESC}[0m"
  [ -n "$rc" ] && PCOL+="${ESC}[38;5;203m${rc}${ESC}[0m"
  PCOL+=" ${ESC}[1;38;5;220m\$${ESC}[0m "; }

hist_prev(){ (( ${#HIST[@]} == 0 )) && return
  (( HISTI == ${#HIST[@]} )) && HSAVE="$LINE"
  (( HISTI > 0 )) && HISTI=$((HISTI-1))
  LINE="${HIST[$HISTI]}"; CURPOS=${#LINE}; }
hist_next(){ (( HISTI >= ${#HIST[@]} )) && return
  HISTI=$((HISTI+1))
  if (( HISTI == ${#HIST[@]} )); then LINE="$HSAVE"; else LINE="${HIST[$HISTI]}"; fi
  CURPOS=${#LINE}; }

kill_word(){ local p="${LINE:0:CURPOS}"; local i=${#p}
  while (( i>0 )) && [ "${p:i-1:1}" = " " ]; do ((i--)); done
  while (( i>0 )) && [ "${p:i-1:1}" != " " ]; do ((i--)); done
  LINE="${p:0:i}${LINE:CURPOS}"; CURPOS=$i; }
word_left(){ local p="${LINE:0:CURPOS}"; local i=${#p}
  while (( i>0 )) && [ "${p:i-1:1}" = " " ]; do ((i--)); done
  while (( i>0 )) && [ "${p:i-1:1}" != " " ]; do ((i--)); done; CURPOS=$i; }
word_right(){ local i=$CURPOS n=${#LINE}
  while (( i<n )) && [ "${LINE:i:1}" = " " ]; do ((i++)); done
  while (( i<n )) && [ "${LINE:i:1}" != " " ]; do ((i++)); done; CURPOS=$i; }

shell_complete(){
  local pre="${LINE:0:CURPOS}"; local i=${#pre} w first=0 m common ch k
  while (( i>0 )); do case "${pre:i-1:1}" in ' '|'|'|';'|'&'|'>'|'<'|'=') break;; esac; ((i--)); done
  w="${pre:i}"
  [[ "${pre:0:i}" =~ ^[[:space:]]*$ ]] && first=1
  local -a M=()
  if (( first )) && [[ "$w" != */* ]]; then
    mapfile -t M < <(compgen -c -- "$w" 2>/dev/null | sort -u | head -300)
  else
    mapfile -t M < <(cd "$CWD" 2>/dev/null && compgen -f -- "$w" 2>/dev/null | sort | head -300)
    local -a M2=(); for m in "${M[@]}"; do
      if [ -d "$CWD/$m" ] || [ -d "$m" ]; then M2+=("$m/"); else M2+=("$m"); fi; done
    M=("${M2[@]:-}"); [ -z "${M[0]:-}" ] && M=()
  fi
  (( ${#M[@]} == 0 )) && return
  if (( ${#M[@]} == 1 )); then
    common="${M[0]}"; [[ "$common" != */ ]] && common+=" "
  else
    common="${M[0]}"
    for m in "${M[@]}"; do
      k=0
      while (( k < ${#common} && k < ${#m} )) && [ "${common:k:1}" = "${m:k:1}" ]; do ((k++)); done
      common="${common:0:k}"
    done
    if [ "$common" = "$w" ]; then
      mk_prompt; sb_push "$PPLAIN$LINE" "$PCOL$(hl "$LINE"; printf '%s' "$HLOUT")"
      local acc=""
      for m in "${M[@]:0:120}"; do
        if (( ${#acc} + ${#m} + 2 > COLS-1 )); then sb_push "$acc" "${ESC}[38;5;110m$acc${ESC}[0m"; acc=""; fi
        acc+="$m  "
      done
      [ -n "$acc" ] && sb_push "$acc" "${ESC}[38;5;110m$acc${ESC}[0m"
      return
    fi
  fi
  LINE="${LINE:0:i}${common}${LINE:CURPOS}"; CURPOS=$(( i + ${#common} )); }

start_job(){ local line="$1" first
  read -r first _ <<< "$line"
  mkdir -p "$RUNDIR" 2>/dev/null; : > "$RUNOUT"; : > "$RUNMETA"; RUNPOS=0; PARTIAL=""; FLOODED=0
  if [[ " $FG_CMDS " == *" $first "* ]]; then          # needs the real terminal
    printf '%s' "$OFF"; [ -n "$STTY_SAVE" ] && stty "$STTY_SAVE" 2>/dev/null
    ( cd "$CWD" 2>/dev/null; eval "$line" </dev/tty >/dev/tty 2>&1
      printf '%s\n%s\n' "$PWD" "$?" > "$RUNMETA" ) || true
    printf '%s' "$ON"; [ -n "$STTY_SAVE" ] && stty -icanon -echo -isig -ixon min 1 time 0 2>/dev/null
    finish_job; return
  fi
  { cd "$CWD" 2>/dev/null
    COLUMNS=$COLS; export COLUMNS                 # the pipe has no width of its own
    ls(){ command ls --color=always -C "$@"; }    # keep the colours a pipe would eat
    dir(){ command dir --color=always "$@"; }
    grep(){ command grep --color=always "$@"; }
    egrep(){ command grep -E --color=always "$@"; }
    fgrep(){ command grep -F --color=always "$@"; }
    diff(){ command diff --color=always "$@"; }
    tree(){ command tree -C "$@"; }
    eval "$line"
    rc=$?
    printf '%s\n%s\n' "$PWD" "$rc" > "$RUNMETA"
  } >"$RUNOUT" 2>&1 </dev/null &
  RUNPID=$!; }

# `tree /` can talk faster than any terminal can listen.  Take a bounded bite
# of the output each frame and only ever show the tail of it: the lines in
# between would have scrolled past before you could read them anyway.
MAXBYTES=65536; MAXLINES=200; FLOODED=0

drain_job(){ local sz chunk rest want n st i
  [ -f "$RUNOUT" ] || return
  sz=$(stat -c %s "$RUNOUT" 2>/dev/null || echo 0)
  (( sz <= RUNPOS )) && return
  want=$(( sz - RUNPOS ))
  if (( want > MAXBYTES )); then          # skip to the end of the torrent
    RUNPOS=$(( sz - MAXBYTES )); want=$MAXBYTES; PARTIAL=""
    (( FLOODED )) || { FLOODED=1; sb_note "  (output is coming faster than he can read it; showing the tail)"; }
  fi
  chunk=$(tail -c "+$((RUNPOS+1))" "$RUNOUT" 2>/dev/null | head -c "$want"; printf X)
  chunk="${chunk%X}"; RUNPOS=$sz
  chunk="$PARTIAL$chunk"; PARTIAL=""
  if [[ "$chunk" == *$'\n' ]]; then rest=""; chunk="${chunk%$'\n'}"
  else rest="${chunk##*$'\n'}"; chunk="${chunk%$'\n'*}"
       [ "$rest" = "$chunk" ] && { chunk=""; }
  fi
  if [ -n "$chunk" ]; then
    local -a L=(); mapfile -t L <<< "$chunk"
    n=${#L[@]}; st=0
    (( n > MAXLINES )) && st=$(( n - MAXLINES ))
    for (( i=st; i<n; i++ )); do sb_out "${L[$i]%$'\r'}"; done
  fi
  PARTIAL="$rest"; }

finish_job(){ local p r
  drain_job
  [ -n "$PARTIAL" ] && { sb_out "$PARTIAL"; PARTIAL=""; }
  if [ -s "$RUNMETA" ]; then
    { read -r p; read -r r; } < "$RUNMETA"
    [ -n "${p:-}" ] && [ -d "$p" ] && CWD="$p"
    LASTRC="${r:-0}"
  else LASTRC=130; fi
  RUNPID=""; mk_prompt; }

shell_enter(){
  local line="$LINE" first rest
  mk_prompt; hl "$line"
  sb_push "$PPLAIN$line" "$PCOL$HLOUT"
  LINE=""; CURPOS=0
  if [ -n "${line// /}" ]; then
    (( ${#HIST[@]} == 0 )) || [ "${HIST[-1]}" != "$line" ] && HIST+=("$line")
  fi
  HISTI=${#HIST[@]}; HSAVE=""
  read -r first rest <<< "$line"
  case "${first:-}" in
    "")      LASTRC=0; return;;
    exit|logout) QUIT=1; return;;
    clear)   SB=(); SBC=(); LASTRC=0; return;;
    skob)    run_cmd "$rest"; [ -n "$NOTE" ] && sb_push "  $NOTE" "${ESC}[38;5;${SK_COL[0]:-$BASECOL}m  $NOTE${ESC}[0m"; NOTE=""; LASTRC=0; return;;
  esac
  start_job "$line"; }

shell_key(){ local ch="$1"
  if [ -n "$RUNPID" ]; then
    case "$ch" in
      $'\003') kill -TERM "$RUNPID" 2>/dev/null; pkill -TERM -P "$RUNPID" 2>/dev/null
               sb_note "^C"; ;;
      *) :;;
    esac
    return
  fi
  case "$ch" in
    ""|$'\n'|$'\r') shell_enter;;
    $'\177'|$'\b') (( CURPOS>0 )) && { LINE="${LINE:0:CURPOS-1}${LINE:CURPOS}"; CURPOS=$((CURPOS-1)); };;
    $'\t') shell_complete;;
    $'\001') CURPOS=0;;
    $'\005') CURPOS=${#LINE};;
    $'\002') (( CURPOS>0 )) && CURPOS=$((CURPOS-1));;
    $'\006') (( CURPOS<${#LINE} )) && CURPOS=$((CURPOS+1));;
    $'\013') LINE="${LINE:0:CURPOS}";;
    $'\025') LINE="${LINE:CURPOS}"; CURPOS=0;;
    $'\027') kill_word;;
    $'\014') SB=(); SBC=();;
    $'\003') LINE=""; CURPOS=0; sb_note "^C";;
    $'\004') [ -z "$LINE" ] && QUIT=1;;
    $'\020') hist_prev;;
    $'\016') hist_next;;
    *) case "$ch" in
         [[:print:]]|[^[:cntrl:]]) LINE="${LINE:0:CURPOS}$ch${LINE:CURPOS}"; CURPOS=$((CURPOS+1));;
       esac;;
  esac; }

if (( SHELL_MODE )); then
  mkdir -p "$RUNDIR" 2>/dev/null
  [ -n "$STTY_SAVE" ] && stty -icanon -echo -isig -ixon min 1 time 0 2>/dev/null
  mk_prompt
  sb_push "  skob is loose in your shell." "${ESC}[1;38;5;${SK_COL[0]:-$BASECOL}m  skob is loose in your shell.${ESC}[0m"
  sb_note "  the letters in the bottom $SOLID lines are solid; he falls through the gaps between them."
  sb_note "  \`skob help\` for his commands, drag him with the mouse, ctrl-d to leave."
fi
# ---------------------------------------------------------------------------

for c in "${CMDS[@]}"; do run_cmd "$c"; done   # whatever you asked for on the way in
NOTE=""

frame=0; next_us=$(now_us)
printf '%s' "$ON"
declare -A ROWTXT=() ROWPLAIN=()
while :; do
  COLS=$(tput cols); ROWS=$(tput lines); PW=$(( COLS*2 )); PH=$(( $(playrows)*4 ))
  RESTS=""; for i in "${!SK_REST[@]}"; do RESTS+="$i:${SK_REST[$i]}:${SK_PHYS[$i]} "; done

  if (( SHELL_MODE )); then
    [ -n "$RUNPID" ] && { drain_job; kill -0 "$RUNPID" 2>/dev/null || finish_job; }
    ROWTXT=(); ROWPLAIN=()
    n=${#SB[@]}; avail=$(( ROWS-1 )); (( avail<1 )) && avail=1
    nvis=$(( n<avail ? n : avail )); TOPTEXT=$(( ROWS-nvis ))
    for (( i=0; i<nvis; i++ )); do
      r=$(( TOPTEXT+i )); ROWTXT[$r]="${SBC[$((n-nvis+i))]}"; ROWPLAIN[$r]="${SB[$((n-nvis+i))]}"
    done
    # the bottom SOLID rows are ground only where they actually have ink
    TLINES=""; SOLIDTOP=$(( (ROWS-SOLID-1)*4 )); (( SOLIDTOP<0 )) && SOLIDTOP=0
    for (( r=ROWS-SOLID; r<=ROWS-1; r++ )); do
      (( r<1 )) && continue
      t="${ROWPLAIN[$r]:-}"
      [ -n "${t// /}" ] && TLINES+="T $((r-1)) $t"$'\n'
    done
  fi
  # a burst of output shoves him up a few rows a frame, not all at once
  PENDPUSH=$(( PENDPUSH + NEWLINES )); NEWLINES=0
  (( PENDPUSH > 200 )) && PENDPUSH=200          # a flood shoves him up, it does not bury him
  step=$(( PENDPUSH > 3 ? 3 : PENDPUSH )); PENDPUSH=$(( PENDPUSH - step ))
  PUSHUP=$(( step*4 )); PUSHTOP=$(( (TOPTEXT-1)*4 ))

  if [ ${#PTS[@]} -gt 0 ]; then
  OUT=$({ printf '%s' "${TLINES:-}"; printf '%s\n' "${PTS[@]}"; } | awk \
      -v G="$GRAV" -v K="$STIFF" -v B="$BOUNCE" -v FR="$FRIC" \
      -v PW="$PW" -v PH="$PH" -v RESTS="$RESTS" -v PAUSED="$PAUSED" \
      -v SOLIDTOP="${SOLIDTOP:-999999}" \
      -v PUSH="$PUSHUP" -v PUSHTOP="$PUSHTOP" \
      -v GS="$GRABS" -v GI="$GRABI" -v GX="$(( (MX-1)*2 ))" -v GY="$(( (MY-1)*4 ))" '
    BEGIN{ PI=3.14159265; nr=split(RESTS,rr," ")          # sid:rest:k:lo:hi:twist:mode
           for(q=1;q<=nr;q++){ split(rr[q],kv,":")
             REST[kv[1]]=kv[2]; KK[kv[1]]=kv[3]; LO[kv[1]]=kv[4]
             HI[kv[1]]=kv[5]; LIM[kv[1]]=kv[6]; MODE[kv[1]]=kv[7]
             SHINE[kv[1]]=kv[8] } }
    $1=="T"{ row=$2+0; txt=substr($0, length($1)+length($2)+3)   # a row of ground
             for(cc=0; cc<length(txt); cc++)
               if(substr(txt,cc+1,1)!=" ") SOL[row,cc]=1
             NSOL=1; next }
    { s=$1; l=$2; x[s,l]=$3; y[s,l]=$4; ox[s,l]=$5; oy[s,l]=$6
      if(l>NP[s]) NP[s]=l; seen[s]=1 }
    END{
      if(PUSH>0){                                   # the rising lines shove him up
       for(s in seen){ N=NP[s]; my=-1e9; mn=1e9; xlo=1e9; xhi=-1e9
        for(i=0;i<=N;i++){ if(y[s,i]>my) my=y[s,i]; if(y[s,i]<mn) mn=y[s,i]
                           if(x[s,i]<xlo) xlo=x[s,i]; if(x[s,i]>xhi) xhi=x[s,i] }
        if(my<=PUSHTOP) continue
        if(!inked(int(my/4), int(xlo/2), int(xhi/2))) continue  # nothing of his is rising
        sh=PUSH; if(sh>mn) sh=mn; if(sh>0)
          for(i=0;i<=N;i++){ y[s,i]-=sh; oy[s,i]-=(sh-1.2) } } }
      if(!PAUSED){
       for(s in seen){ N=NP[s]                        # N = perimeter count, index N = middle
        if(MODE[s]>=2) continue                       # a grain keeps to its own rules
        for(i=0;i<=N;i++){
          if(s==GS && i==GI){ ox[s,i]=x[s,i]; oy[s,i]=y[s,i]
                              x[s,i]+=(GX-x[s,i])*0.45; y[s,i]+=(GY-y[s,i])*0.45; continue }
          vx=(x[s,i]-ox[s,i])*FR; vy=(y[s,i]-oy[s,i])*FR
          sp=sqrt(vx*vx+vy*vy); MAXV=REST[s]*(MODE[s]==0?0.7:3.0)
          if(sp>MAXV){ vx*=MAXV/sp; vy*=MAXV/sp }
          ox[s,i]=x[s,i]; oy[s,i]=y[s,i]
          x[s,i]+=vx; y[s,i]+=vy+G }
        for(pass=0; pass<6; pass++){
         if(MODE[s]==1){                                # a chain: just the links
          for(i=0;i<N;i++) solve(s,i,i+1,REST[s],1.0)
         } else {
          for(i=0;i<N;i++){ j=(i+1)%N
            solve(s,i,j,REST[s]*2*sin(PI/N),KK[s]); solve(s,i,N,REST[s],KK[s]) }
          for(i=0;i<N;i++){                            # hard shell
            px=x[s,i]-x[s,N]; py=y[s,i]-y[s,N]; pd=sqrt(px*px+py*py)
            if(pd<0.000001){ px=1; py=0; pd=1 }
            lo=REST[s]*LO[s]; hi=REST[s]*HI[s]
            if(pd<lo){ x[s,i]=x[s,N]+px/pd*lo; y[s,i]=y[s,N]+py/pd*lo }
            else if(pd>hi){ x[s,i]=x[s,N]+px/pd*hi; y[s,i]=y[s,N]+py/pd*hi } }
          sc=0; ss=0                                    # allow spin: find the mean twist
          for(i=0;i<N;i++){ ang[i]=atan2(y[s,i]-y[s,N], x[s,i]-x[s,N])
            dfa=ang[i]-2*PI*i/N; sc+=cos(dfa); ss+=sin(dfa) }
          off=atan2(ss,sc)
          for(i=0;i<N;i++){                             # ...then pin each to its wedge
            nom=2*PI*i/N+off; dfa=ang[i]-nom
            while(dfa>PI) dfa-=2*PI
            while(dfa<-PI) dfa+=2*PI
            lim=PI/N*LIM[s]
            if(dfa>lim) dfa=lim; else if(dfa<-lim) dfa=-lim
            rq=sqrt((x[s,i]-x[s,N])^2+(y[s,i]-y[s,N])^2)
            x[s,i]=x[s,N]+rq*cos(nom+dfa); y[s,i]=y[s,N]+rq*sin(nom+dfa) }
         }
          for(i=0;i<=N;i++){                            # walls
            if(x[s,i]<0){ x[s,i]=0; ox[s,i]=x[s,i]+(ox[s,i]-x[s,i])*B }
            if(x[s,i]>PW-1){ x[s,i]=PW-1; ox[s,i]=x[s,i]+(ox[s,i]-x[s,i])*B }
            if(y[s,i]<0){ y[s,i]=0; oy[s,i]=y[s,i]+(oy[s,i]-y[s,i])*B }
            if(y[s,i]>PH-1){ y[s,i]=PH-1; oy[s,i]=y[s,i]+(oy[s,i]-y[s,i])*B }
            if(pushout(x[s,i],y[s,i])){               # down there the letters are rock
              x[s,i]+=RESX; y[s,i]+=RESY
              if(RESY!=0) oy[s,i]=y[s,i]+(oy[s,i]-y[s,i])*B
              else        ox[s,i]=x[s,i]+(ox[s,i]-x[s,i])*B } }
          for(i=0;i<N;i++){ j=(i+1)%N                    # and the skin between them,
            for(t=1;t<=3;t++){                           # so no letter slips through
              u=t/4; sx=x[s,i]+(x[s,j]-x[s,i])*u; sy=y[s,i]+(y[s,j]-y[s,i])*u
              if(pushout(sx,sy)){
                x[s,i]+=RESX*(1-u); y[s,i]+=RESY*(1-u)
                x[s,j]+=RESX*u;     y[s,j]+=RESY*u
                if(RESY!=0){ oy[s,i]=y[s,i]+(oy[s,i]-y[s,i])*B; oy[s,j]=y[s,j]+(oy[s,j]-y[s,j])*B }
                else       { ox[s,i]=x[s,i]+(ox[s,i]-x[s,i])*B; ox[s,j]=x[s,j]+(ox[s,j]-x[s,j])*B } } } } } }
       # skobs shove each other apart instead of overlapping
       for(a in seen) for(b in seen){ if(a>=b) continue
         if(MODE[a]>=1 || MODE[b]>=1) continue          # only bodies shove bodies
         Na=NP[a]; Nb=NP[b]
         dx=x[b,Nb]-x[a,Na]; dy=y[b,Nb]-y[a,Na]; d=sqrt(dx*dx+dy*dy)
         want=(REST[a]+REST[b])*0.92
         if(d>0.001 && d<want){ push=(want-d)/d*0.5
           wa=REST[b]/(REST[a]+REST[b]); wb=1-wa      # the small one gives way
           for(i=0;i<=Na;i++){ x[a,i]-=dx*push*wa; y[a,i]-=dy*push*wa }
           for(i=0;i<=Nb;i++){ x[b,i]+=dx*push*wb; y[b,i]+=dy*push*wb } } }
       ng=0; nb=0                                     # ---- the sandbox ----
       for(s in seen){ if(MODE[s]>=2) gr[ng++]=s; else if(MODE[s]==0) bl[nb++]=s }
       if(ng>0){
         for(q=0;q<ng;q++) for(w=q+1;w<ng;w++)         # the lowest grain moves first,
           if(y[gr[w],0]>y[gr[q],0]){ t2=gr[q]; gr[q]=gr[w]; gr[w]=t2 }   # so piles settle
         delete OCC
         for(q=0;q<ng;q++){ gsnap(gr[q]); gmark(gr[q],1) }
         for(q=0;q<ng;q++){ s=gr[q]
           gw=(MODE[s]==3?2:1); gh=(MODE[s]==3?4:1)
           gmark(s,0)                                  # lift it off the board first
           gx2=x[s,0]; gy2=y[s,0]
           if(gblocked(s,gx2,gy2)){                    # something sat on it: get out
                  if(!gblocked(s,gx2,gy2-gh))   gy2-=gh
             else if(!gblocked(s,gx2-gw,gy2))   gx2-=gw
             else if(!gblocked(s,gx2+gw,gy2))   gx2+=gw
             else if(!gblocked(s,gx2,gy2-2*gh)) gy2-=2*gh }
           for(rp=0; rp<(MODE[s]==3?1:3); rp++){       # fall, then try to slide
             if(!gblocked(s,gx2,gy2+gh)){ gy2+=gh; continue }
             gd=(rand()<0.5?-gw:gw)
             if(!gblocked(s,gx2+gd,gy2+gh)){ gx2+=gd; gy2+=gh }
             else if(!gblocked(s,gx2-gd,gy2+gh)){ gx2-=gd; gy2+=gh }
             else break }
           x[s,0]=gx2; y[s,0]=gy2; ox[s,0]=gx2; oy[s,0]=gy2
           gmark(s,1) } }
      }
      no=0; for(s in seen) ord[no++]=s+0          # always draw them in the same order,
      for(q=0;q<no;q++) for(w=q+1;w<no;w++)        # or the overlaps flicker colour
        if(ord[w]<ord[q]){ t2=ord[q]; ord[q]=ord[w]; ord[w]=t2 }
      for(oi=0;oi<no;oi++){ s=ord[oi]; N=NP[s]
        for(i=0;i<=N;i++) printf "P %s %d %.3f %.3f %.3f %.3f\n", s,i,x[s,i],y[s,i],ox[s,i],oy[s,i]
        if(MODE[s]==2){ printf "X %s %d %d\n", s,int(x[s,0]),int(y[s,0]); continue }
        if(MODE[s]==3){ printf "B %s %d %d\n", s,int(x[s,0]/2),int(y[s,0]/4); continue }
        if(MODE[s]==1){                               # walk the links, dot by dot
          for(i=0;i<N;i++){ ax=x[s,i]; ay=y[s,i]; bx=x[s,i+1]; by=y[s,i+1]
            st=int((bx>ax?bx-ax:ax-bx)+(by>ay?by-ay:ay-by))+1
            for(q=0;q<=st;q++) printf "X %s %d %d\n", s,int(ax+(bx-ax)*q/st),int(ay+(by-ay)*q/st) }
          continue }
        miny=1e9; maxy=-1e9
        for(i=0;i<N;i++){ if(y[s,i]<miny)miny=y[s,i]; if(y[s,i]>maxy)maxy=y[s,i] }
        for(sy=int(miny); sy<=int(maxy); sy++){ c=0
          for(i=0;i<N;i++){ j=(i+1)%N
            if((y[s,i]<=sy && y[s,j]>sy) || (y[s,j]<=sy && y[s,i]>sy))
              xs[c++]=x[s,i]+(sy-y[s,i])/(y[s,j]-y[s,i])*(x[s,j]-x[s,i]) }
          for(a=0;a<c-1;a++) for(b2=a+1;b2<c;b2++) if(xs[b2]<xs[a]){ t=xs[a];xs[a]=xs[b2];xs[b2]=t }
          for(a=0;a+1<c;a+=2) for(px2=int(xs[a]); px2<=int(xs[a+1]); px2++)
            printf "X %s %d %d\n", s,px2,sy }
        if(SHINE[s]){        # a curved sweep of light, up and to the left,
          ar=REST[s]*0.62         # sized to the thing it is sitting on
          th=int(REST[s]/7+0.5); if(th<1) th=1; if(th>3) th=3
          steps=int(ar*2.1); if(steps<6) steps=6
          for(q3=0;q3<=steps;q3++){ aa=-2.25-0.525+1.05*q3/steps
            for(tt=0;tt<th;tt++)
              printf "H %s %d %d\n", s,int(x[s,N]+(ar-tt)*cos(aa)),int(y[s,N]+(ar-tt)*sin(aa)) } }
        printf "C %s %.3f %.3f\n", s,x[s,N],y[s,N] }
    }
    # is there any ink at all below him, in the columns he stands over?  if not
    # he is over bare screen and the scrolling has nothing to do with him.
    function inked(r0,c0,c1,   r,c){
      if(!NSOL) return 0
      for(r=r0; r<PH/4; r++) for(c=c0; c<=c1; c++) if((r,c) in SOL) return 1
      return 0 }

    # a grain sits on the dot grid, or on the character grid if it is a big one
    function gsnap(s,   w,h){ w=(MODE[s]==3?2:1); h=(MODE[s]==3?4:1)
      if(MODE[s]==3){ x[s,0]=int(x[s,0]/2)*2; y[s,0]=int(y[s,0]/4)*4 }
      else          { x[s,0]=int(x[s,0]);     y[s,0]=int(y[s,0]) }
      if(x[s,0]<0) x[s,0]=0; if(y[s,0]<0) y[s,0]=0
      if(x[s,0]>PW-w) x[s,0]=int((PW-w)/w)*w
      if(y[s,0]>PH-h) y[s,0]=int((PH-h)/h)*h }
    function gmark(s,on,   i,j,w,h){ w=(MODE[s]==3?2:1); h=(MODE[s]==3?4:1)
      for(i=0;i<w;i++) for(j=0;j<h;j++){
        if(on) OCC[x[s,0]+i,y[s,0]+j]=1; else delete OCC[x[s,0]+i,y[s,0]+j] } }
    # nothing may share the spot of a grain: no other grain, no letter, no skob
    function gblocked(s,px,py,   i,j,w,h,cr,cc,k,b,dx,dy,rr){
      w=(MODE[s]==3?2:1); h=(MODE[s]==3?4:1)
      if(px<0 || px+w>PW || py+h>PH) return 1
      for(i=0;i<w;i++) for(j=0;j<h;j++){
        if((px+i,py+j) in OCC) return 1
        if(NSOL && py+j>=SOLIDTOP){ cr=int((py+j)/4); cc=int((px+i)/2)
          if((cr,cc) in SOL) return 1 } }
      for(k=0;k<nb;k++){ b=bl[k]; rr=REST[b]*0.95
        dx=px+w/2-x[b,NP[b]]; dy=py+h/2-y[b,NP[b]]
        if(dx*dx+dy*dy < rr*rr) return 1 }
      return 0 }

    # a point inside a letter leaves by its nearest free edge -- falling, that
    # is the top, so he sits on the letter; walking, it is the side, so he stops.
    function pushout(px,py,   cr,cc,lx,ty,dl,dr,dt,db,best){
      RESX=0; RESY=0
      if(!NSOL || py<SOLIDTOP) return 0
      cr=int(py/4); cc=int(px/2)
      if(!((cr,cc) in SOL)) return 0
      lx=cc*2; ty=cr*4
      dl=px-lx+0.02;   if((cr,cc-1) in SOL || cc<=0)   dl=1e9
      dr=lx+2-px+0.02; if((cr,cc+1) in SOL || lx+2>=PW) dr=1e9
      dt=py-ty+0.02;   if((cr-1,cc) in SOL)            dt=1e9
      db=ty+4-py+0.02; if((cr+1,cc) in SOL || ty+4>=PH) db=1e9
      best=dt; RESY=-dt
      if(dl<best){ best=dl; RESX=-dl; RESY=0 }
      if(dr<best){ best=dr; RESX=dr;  RESY=0 }
      if(db<best){ best=db; RESX=0;   RESY=db }
      if(best>=1e9){ RESX=0; RESY=-(py-ty+0.02) }      # walled in: up regardless
      return 1
    }
    function solve(s,a,b,rest,kk,   dx,dy,d,diff,mx,my){
      dx=x[s,b]-x[s,a]; dy=y[s,b]-y[s,a]; d=sqrt(dx*dx+dy*dy); if(d==0) return
      diff=(d-rest)/d*0.5*K*kk; mx=dx*diff; my=dy*diff
      if(!(s==GS && a==GI)){ x[s,a]+=mx; y[s,a]+=my }
      if(!(s==GS && b==GI)){ x[s,b]-=mx; y[s,b]-=my }
    }')
  else OUT=""; fi

  PTS=(); declare -A CAN=() CCOL=() CGL=() HL=(); declare -A CENX=() CENY=()
  while read -r tag a b c d e; do
    case "$tag" in
      P) PTS+=("$a $b $c $d $e");;
      X) px=$b; py=$c
         (( px<0 || py<0 || px>=PW || py>=PH )) && continue
         key="$(( py/4 )),$(( px/2 ))"
         CAN[$key]=$(( ${CAN[$key]:-0} | DOT[$(( (px%2)*4 + py%4 ))] )); CCOL[$key]="${SK_COL[$a]:-$BASECOL}";;
      H) px=$b; py=$c                                            # a dot of the highlight
         (( px<0 || py<0 || px>=PW || py>=PH )) && continue
         key="$(( py/4 )),$(( px/2 ))"
         CAN[$key]=$(( ${CAN[$key]:-0} | DOT[$(( (px%2)*4 + py%4 ))] ))
         CCOL[$key]="${SK_COL[$a]:-$BASECOL}"; HL[$key]=1;;
      B) (( b<0 || c<0 || b>=COLS || c>=ROWS )) && continue      # a grain with a letter of its own
         key="$c,$b"; CAN[$key]=0; CGL[$key]="${SK_GLYPH[$a]:-#}"
         CCOL[$key]="${SK_COL[$a]:-$BASECOL}";;
      C) CENX[$a]=${b%.*}; CENY[$a]=${c%.*};;
    esac
  done <<< "$OUT"

  buf="${ESC}[H${ESC}[2J"

  if (( SHELL_MODE )); then                       # the scrollback he haunts
    for (( r=TOPTEXT; r<ROWS; r++ )); do
      [ -n "${ROWTXT[$r]:-}" ] && buf+="${ESC}[${r};1H${ROWTXT[$r]}${ESC}[0m"
    done
  fi

  for key in "${!CAN[@]}"; do
    cy=${key%,*}; cx=${key#*,}
    if (( SHELL_MODE )); then
      pline="${ROWPLAIN[$((cy+1))]:-}"; pch="${pline:$cx:1}"
      if [ -n "$pch" ] && [ "$pch" != " " ]; then
        # down in the solid rows a letter is rock: it stays its own colour and he
        # stays behind it.  higher up he is a ghost and wears the text he is on.
        (( cy+1 >= ROWS-SOLID )) && continue
        buf+="${ESC}[$((cy+1));$((cx+1))H${ESC}[0;1;38;5;${CCOL[$key]}m${pch}"
        continue
      fi
    fi
    if [ -n "${CGL[$key]:-}" ]; then
      buf+="${ESC}[$((cy+1));$((cx+1))H${ESC}[0;1;38;5;${CCOL[$key]}m${CGL[$key]}"
    else
      ccol="${CCOL[$key]}"
      [ -n "${HL[$key]:-}" ] && { lighten "$ccol"; ccol="$LITOUT"; }
      buf+="${ESC}[$((cy+1));$((cx+1))H${ESC}[0;38;5;${ccol}m${BR[${CAN[$key]}]}"
    fi
  done
  buf+="${ESC}[0m"
  for sid in "${!CENX[@]}"; do
    ecx=$(( CENX[$sid]/2 + 1 )); ecy=$(( CENY[$sid]/4 + 1 ))
    [ "${SK_EYES[$sid]:-1}" = 1 ] || continue     # objects are only objects
    if (( MX > 0 )); then dx=$(( MX>ecx ? 1 : (MX<ecx ? -1 : 0) )); dy=$(( MY>ecy ? 1 : (MY<ecy ? -1 : 0) ))
    else dx=0; dy=0; fi
    buf+="${ESC}[$((ecy+dy));$((ecx-1+dx))H${ESC}[1m${ESC}[38;5;${EYECOL}m◉${ESC}[0m"
    buf+="${ESC}[$((ecy+dy));$((ecx+2+dx))H${ESC}[1m${ESC}[38;5;${EYECOL}m◉${ESC}[0m"
  done

  if [ -n "$PLACING" ]; then                       # you cannot miss what you are doing
    if (( SHELL_MODE )); then bnr=" PLACING $PLACING · \`skob stop\` TO STOP "
    else bnr=" PLACING $PLACING · :stop TO STOP "; fi
    buf+="${ESC}[1;1H${ESC}[0;1;7;38;5;220m${bnr:0:$((COLS-1))}${ESC}[0m"
  fi

  if (( SHELL_MODE )); then
    mk_prompt
    if [ -n "$RUNPID" ]; then
      buf+="${ESC}[${ROWS};1H${ESC}[K${ESC}[38;5;220m${SPIN[$((frame%8))]}${ESC}[0m ${ESC}[38;5;244mrunning · ^C stops it${ESC}[0m"
      curcol=1
    else
      plen=${#PPLAIN}; avail=$(( COLS-plen-1 )); (( avail<10 )) && avail=10
      off=0; (( CURPOS > avail )) && off=$(( CURPOS-avail ))
      vis="${LINE:off:avail}"; hl "$vis"
      buf+="${ESC}[${ROWS};1H${ESC}[K${PCOL}${HLOUT}"
      curcol=$(( plen + (CURPOS-off) + 1 ))
    fi
    buf+="${ESC}[${ROWS};${curcol}H"
  elif [ "$MODE" = cmd ]; then
    buf+="${ESC}[${ROWS};1H${ESC}[K${ESC}[38;5;220m:${CMD}${ESC}[7m ${ESC}[0m"
  else
    nth=${#SK_COL[@]}; word=skob
    for k in ${SK_KIND+"${SK_KIND[@]}"}; do [ "$k" = skob ] || { word=thing; break; }; done
    st=" $nth $word$([ "$nth" -ne 1 ] && echo s) · gravity ${GRAV} · stiffness ${STIFF} · $([ $PAUSED = 1 ] && echo held || echo squishing) ${NOTE:+· $NOTE} · ; for commands "
    buf+="${ESC}[${ROWS};1H${ESC}[7m${st:0:$((COLS-1))}${ESC}[0m"
  fi
  printf '%s' "$buf"

  frame=$((frame+1))
  [ "$FRAMES" -gt 0 ] && [ "$frame" -ge "$FRAMES" ] && break

  next_us=$(( next_us + TICK_US )); cur=$(now_us)
  (( next_us < cur )) && next_us=$(( cur + TICK_US ))
  while :; do
    cur=$(now_us); rem=$(( next_us - cur )); (( rem <= 0 )) && break
    rs=$(printf '%d.%06d' $((rem/1000000)) $((rem%1000000)))
    IFS= read -rsn1 -t "$rs" ch 2>/dev/null || break
    if [ "$ch" = "$ESC" ]; then
      # A whole escape sequence or none of it.  Time a continuation byte out too
      # tightly and the tail of a mouse report -- <32;49;19M -- is left behind to
      # be typed into your command line as if you had meant it.
      IFS= read -rsn1 -t 0.06 c2 2>/dev/null || {
        (( SHELL_MODE )) || { [ "$MODE" = cmd ] && { MODE=normal; CMD=""; }; }; continue; }
      if [ "$c2" = "O" ]; then IFS= read -rsn1 -t 0.06 c3 2>/dev/null; seq="${c3:-}"
      elif [ "$c2" = "[" ]; then
        seq=""
        while IFS= read -rsn1 -t 0.12 c3 2>/dev/null; do
          seq+="$c3"
          # a mouse report only ever ends at M or m, so keep reading until one
          case "$seq" in
            '<'*) case "$c3" in [Mm]) break;; esac;;
            *)    case "$c3" in [A-Za-z~]) break;; esac;;
          esac
          (( ${#seq} > 32 )) && break
        done
      else continue; fi
      # whatever it was, if it still looks like a mouse report, it is not typing
      case "$seq" in '<'*[Mm]) :;; '<'*) continue;; esac
      if [[ "$seq" =~ ^\<([0-9]+)\;([0-9]+)\;([0-9]+)([Mm])$ ]]; then
        btn="${BASH_REMATCH[1]}"; MX="${BASH_REMATCH[2]}"; MY="${BASH_REMATCH[3]}"; act="${BASH_REMATCH[4]}"
        if [ "$act" = "M" ] && (( btn == 0 )) && [ -n "$PLACING" ]; then
          place_at $(( (MX-1)*2 )) $(( (MY-1)*4 )) "$PLACING" "$PLACEN"
        elif [ "$act" = "M" ] && (( btn == 0 )); then
          gx=$(( (MX-1)*2 )); gy=$(( (MY-1)*4 )); bd=-1; GRABS=-1; GRABI=-1
          for p in "${PTS[@]}"; do
            read -r sid li x y _ <<< "$p"
            (( li >= ${SK_N[$sid]:-99} )) && continue
            (( ${SK_MODE[$sid]:-0} >= 2 )) && continue    # you cannot grab a grain
            d=$(awk -v x="$x" -v y="$y" -v a="$gx" -v b="$gy" 'BEGIN{printf "%d",(x-a)^2+(y-b)^2}')
            # every point inside a body is within a radius of some point of its
            # skin, so this reach makes the whole of it a handle, not just the rim
            rr=${SK_REST[$sid]:-8}; lim=$(( rr*rr*121/100 )); (( lim < 900 )) && lim=900
            (( d > lim )) && continue
            (( bd < 0 || d < bd )) && { bd=$d; GRABS=$sid; GRABI=$li; }
          done
          [ "$GRABS" != -1 ] && NOTE="held"
        elif [ "$act" = "m" ]; then GRABS=-1; GRABI=-1
        fi
      elif (( SHELL_MODE )) && [ -z "$RUNPID" ]; then
        case "$seq" in
          A) hist_prev;; B) hist_next;;
          C) (( CURPOS < ${#LINE} )) && CURPOS=$((CURPOS+1));;
          D) (( CURPOS > 0 )) && CURPOS=$((CURPOS-1));;
          H|'1~'|'7~') CURPOS=0;;
          F|'4~'|'8~') CURPOS=${#LINE};;
          '3~') LINE="${LINE:0:CURPOS}${LINE:CURPOS+1}";;
          '1;5C'|'1;3C') word_right;;
          '1;5D'|'1;3D') word_left;;
        esac
      fi
      continue
    fi
    if (( SHELL_MODE )); then shell_key "$ch"
    elif [ "$MODE" = cmd ]; then
      case "$ch" in
        $'\003') MODE=normal; CMD="";;                     # ^C: never mind
        ""|$'\n'|$'\r') run_cmd "$CMD"; MODE=normal; CMD="";;
        $'\177'|$'\b') CMD="${CMD%?}";;
        *) CMD+="$ch";;
      esac
    else
      case "$ch" in
        ';'|':') MODE=cmd; CMD="";;
        q|$'\003'|$'\004') QUIT=1;;        # q, ^C or ^D all mean leave
        ' ') PAUSED=$((1-PAUSED));;
        g) GRAV=$(awk -v g="$GRAV" 'BEGIN{print (g>0? 0 : 0.32)}');;
        b) BOUNCE=$(awk -v b="$BOUNCE" 'BEGIN{b+=0.15; print (b>0.95?0.95:b)}');;
        s) STIFF=$(awk -v k="$STIFF" 'BEGIN{k-=0.12; print (k<0.22?0.22:k)}');;
        h) STIFF=$(awk -v k="$STIFF" 'BEGIN{k+=0.12; print (k>0.95?0.95:k)}');;
        r) reset_all;;
      esac
    fi
    [ "$QUIT" = 1 ] && break
  done
  [ "$QUIT" = 1 ] && break
done
cleanup
