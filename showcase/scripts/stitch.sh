#!/usr/bin/env bash
set -euo pipefail
S=/tmp/bifrost-show; C=$S/clips; mkdir -p "$C"
F=/nix/store/s0v7cizdm8c5d3v1z9467lw33q1ksq44-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf
PORTAL=$(ls $S/vid-portal/*.webm | head -1)
GITHUB=$(ls $S/vid-github/*.webm | head -1)
BG=0x1d2021; YEL=0xfabd2f; FG=0xebdbb2; AQUA=0x8ec07c; GRAY=0x928374; ORANGE=0xfe8019
R=30; SAR="anullsrc=r=44100:cl=stereo"
VENC=(-c:v libx264 -pix_fmt yuv420p -r $R -preset medium -crf 20 -c:a aac -shortest -movflags +faststart)

dt(){ # text x y size color  -> drawtext fragment
  echo "drawtext=fontfile=$F:text='$1':x=$2:y=$3:fontsize=$4:fontcolor=$5"
}

# --- card: title line + subtitle line(s) ---
card(){ # out dur "BIG" "sub1" "sub2" bigsize
  local out=$1 dur=$2 big=$3 s1=$4 s2=$5 bs=${6:-72}
  local f="color=c=$BG:s=1280x720:r=$R:d=$dur"
  f="$f,$(dt "$big" "(w-text_w)/2" "300" "$bs" "$YEL")"
  [ -n "$s1" ] && f="$f,$(dt "$s1" "(w-text_w)/2" "410" 34 "$FG")"
  [ -n "$s2" ] && f="$f,$(dt "$s2" "(w-text_w)/2" "460" 28 "$AQUA")"
  ffmpeg -y -f lavfi -i "$f" -f lavfi -i "$SAR" "${VENC[@]}" -t "$dur" "$out" >/dev/null 2>&1
}

# --- content clip with a lower-third caption ---
clip(){ # out input speed caption  [extra vf prefix]
  local out=$1 in=$2 spd=$3 cap=$4 pre=${5:-}
  local vf="${pre}scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=$BG,setpts=PTS/$spd"
  vf="$vf,drawbox=x=0:y=h-66:w=iw:h=66:color=black@0.55:t=fill"
  vf="$vf,$(dt "$cap" 40 "h-46" 26 "$FG")"
  ffmpeg -y -i "$in" -f lavfi -i "$SAR" -vf "$vf" "${VENC[@]}" "$out" >/dev/null 2>&1
}

echo "1/7 intro"; card $C/00.mp4 4.5 "BIFROST" "Azure DevOps  to  GitHub Actions — at portfolio scale" "Wrap the Importer · review-first · gap-aware · attestable" 96
echo "2/7 sec1";  card $C/01.mp4 2.6 "1 — Audit & convert" "The official Importer does the bulk; Bifrost finds the gaps" "" 60
echo "3/7 term";  clip $C/02.mp4 "$S/term.gif" 1.25 "CLI — real Azure DevOps audit, real conversion, real PRs"
echo "4/7 sec2";  card $C/03.mp4 2.6 "2 — Review" "A portfolio heatmap, a 3-pane diff, a program board" "" 60
echo "5/7 portal";clip $C/04.mp4 "$PORTAL" 1.0 "Live portal — 6 pipelines across 3 projects · gaps & risk explained"
echo "6/7 sec3";  card $C/05.mp4 2.6 "3 — The result" "Public on GitHub — repos, reviewed PRs, a migration board" "" 60
echo "7/7 github";clip $C/06.mp4 "$GITHUB" 2.0 "Bifrost-opened PRs · converted workflows · public program board"
card $C/07.mp4 8 "Review it yourself" "github.com/olafkfreund/contoso-payments  ·  northwind-logistics  ·  fabrikam-identity" "board — github.com/users/olafkfreund/projects/8" 52

ls -1 $C/0?.mp4 | sed "s/^/file '/;s/$/'/" > $C/list.txt
echo "=== concat ==="
ffmpeg -y -f concat -safe 0 -i $C/list.txt -c copy $S/bifrost-showcase.mp4 >/dev/null 2>&1
echo "done: $S/bifrost-showcase.mp4"
ffprobe -v error -show_entries format=duration:stream=width,height -of default=noprint_wrappers=1 $S/bifrost-showcase.mp4 2>/dev/null | head
ls -la $S/bifrost-showcase.mp4 | awk '{print "bytes:",$5}'
