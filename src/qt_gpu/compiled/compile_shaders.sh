#!/bin/bash
QSB='../../../ext/6.3.2/msvc2019_64/bin/qsb.exe --glsl "100 es,200 es,300 es,330,400,120,100" --hlsl 50 --msl 12'

DISTORTION_MODELS=( "opencv_fisheye" "opencv_standard" "poly3" "poly5" "ptlens" )

for i in "${DISTORTION_MODELS[@]}"
do
    echo "#version 420" > tmp.frag
    cat ../../core/stabilization/distortion_models/gopro_superview.glsl >> tmp.frag
    cat ../../core/stabilization/distortion_models/$i.glsl ../undistort.frag >> tmp.frag
    eval "$QSB -o undistort_$i.frag.qsb tmp.frag"
    rm tmp.frag
done

eval "$QSB -o texture.vert.qsb ../texture.vert"
