@echo off
..\..\..\ext\6.3.0\msvc2019_64\bin\qsb.exe --glsl "100 es,200 es,300 es,330,400,120,100" --hlsl 50 --msl 12 -o undistort.frag.qsb ../undistort.frag
..\..\..\ext\6.3.0\msvc2019_64\bin\qsb.exe --glsl "100 es,200 es,300 es,330,400,120,100" --hlsl 50 --msl 12 -o texture.vert.qsb ../texture.vert