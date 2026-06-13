#!/usr/bin/env bash
set -euo pipefail
S=/tmp/bifrost-show; C=$S/clips
F=/nix/store/s0v7cizdm8c5d3v1z9467lw33q1ksq44-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf
BG=0x1d2021; YEL=0xfabd2f; FG=0xebdbb2; AQUA=0x8ec07c; GRAY=0x928374; R=30
dt(){ echo "drawtext=fontfile=$F:text='$1':x=(w-text_w)/2:y=$2:fontsize=$3:fontcolor=$4"; }
vf="color=c=$BG:s=1280x720:r=$R:d=8"
vf="$vf,$(dt 'Review it yourself' 250 84 $YEL)"
vf="$vf,$(dt '3 public repositories under github.com/olafkfreund' 384 30 $FG)"
vf="$vf,$(dt 'contoso-payments   northwind-logistics   fabrikam-identity' 430 30 $AQUA)"
vf="$vf,$(dt 'program board  github.com/users/olafkfreund/projects/8' 480 26 $GRAY)"
ffmpeg -y -f lavfi -i "$vf" -f lavfi -i "anullsrc=r=44100:cl=stereo" \
  -c:v libx264 -pix_fmt yuv420p -r $R -preset medium -crf 20 -c:a aac -shortest -movflags +faststart -t 8 "$C/07.mp4"
ffmpeg -y -f concat -safe 0 -i "$C/list.txt" -c copy "$S/bifrost-showcase.mp4"
echo "OK dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$S/bifrost-showcase.mp4")"
