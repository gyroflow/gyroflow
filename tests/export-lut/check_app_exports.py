#!/usr/bin/env python3
"""Manually verify lossless neutral/colored full-app exports with real FFmpeg.

Use disposable outputs of the same supplied project, stabilization and codec.
Fixtures and generated media stay outside the repository. This is an explicit
integration check; it does not export media or run during default unit tests.
"""
import argparse
import json
import math
import shutil
import subprocess
from pathlib import Path


def run(command, cwd):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'{Path(command[0]).name} failed ({result.returncode}): {result.stderr[-2000:]}')
    return result.stdout


def probe(ffprobe, media, cwd):
    entries = ('stream=codec_name,width,height,pix_fmt,r_frame_rate,avg_frame_rate,'
               'nb_frames,duration,color_space,color_range,color_transfer,color_primaries:'
               'format=duration:format_tags=comment')
    return json.loads(run([str(ffprobe), '-v', 'error', '-select_streams', 'v:0',
                           '-show_entries', entries, '-of', 'json', str(media)], cwd))


def hashes(path):
    lines = path.read_text(encoding='utf-8').splitlines()
    time_base = [line for line in lines if line.startswith('#tb')]
    frames = [tuple(part.strip() for part in line.split(',')) for line in lines
              if line and not line.startswith('#')]
    if not time_base or not frames or any(len(frame) != 6 for frame in frames):
        raise RuntimeError(f'Incomplete frame hashes: {path.name}')
    return time_base, frames


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ffmpeg_bin', type=Path)
    parser.add_argument('neutral', type=Path)
    parser.add_argument('colored', type=Path)
    parser.add_argument('lut', type=Path)
    parser.add_argument('output_dir', type=Path)
    parser.add_argument('--brightness', type=float, default=.1)
    parser.add_argument('--contrast', type=float, default=.2)
    parser.add_argument('--expected-frames', type=int, required=True)
    args = parser.parse_args()
    for value in (args.brightness, args.contrast):
        if not math.isfinite(value) or abs(value) > .5:
            parser.error('Adjustments must be finite and within -0.5 through 0.5.')
    if args.expected_frames <= 0:
        parser.error('Expected frame count must be positive.')
    neutral, colored, lut = (path.resolve(strict=True) for path in
                              (args.neutral, args.colored, args.lut))
    suffix = '.exe' if (args.ffmpeg_bin / 'ffmpeg.exe').exists() else ''
    ffmpeg = (args.ffmpeg_bin / ('ffmpeg' + suffix)).resolve(strict=True)
    ffprobe = (args.ffmpeg_bin / ('ffprobe' + suffix)).resolve(strict=True)
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(lut, output / 'reference.cube')
    gain, offset = 1 + args.contrast, .5 + args.brightness
    channels = ':'.join(f"{channel}='clip(({channel}(X,Y)-0.5)*{gain}+{offset},0,1)'"
                        for channel in ('r', 'g', 'b'))
    filters = ("format=gbrpf32le,lut3d=file=reference.cube:interp=tetrahedral,"
               f"geq={channels}:a='alpha(X,Y)':interpolation=nearest,format=yuv420p10le")
    common = [str(ffmpeg), '-v', 'error', '-xerror', '-n']
    for media, name, filter_args in ((neutral, 'reference', ['-vf', filters]),
                                     (colored, 'colored', [])):
        run(common + ['-i', str(media), '-map', '0:v:0', '-an'] + filter_args +
            ['-pix_fmt', 'yuv420p10le', '-c:v', 'rawvideo', '-f', 'framehash',
             '-hash', 'sha256', str(output / (name + '.framehash'))], output)
    reference = hashes(output / 'reference.framehash')
    actual = hashes(output / 'colored.framehash')
    if len(reference[1]) != args.expected_frames or len(actual[1]) != args.expected_frames:
        raise RuntimeError('Decoded frame count does not match the complete supplied clip.')
    if reference != actual:
        mismatch = sum(a != b for a, b in zip(reference[1], actual[1]))
        raise RuntimeError(f'Export comparison differs in {mismatch} frames or in time base.')
    probes = [probe(ffprobe, media, output) for media in (neutral, colored)]
    if len(probes[0]['streams']) != 1 or len(probes[1]['streams']) != 1:
        raise RuntimeError('Expected one selected video stream.')
    if probes[0]['streams'][0] != probes[1]['streams'][0]:
        raise RuntimeError('Neutral/colored stream geometry, timing or color properties differ.')
    if probes[1]['streams'][0]['pix_fmt'] != 'yuv420p10le':
        raise RuntimeError('Expected ten-bit YUV420 output.')
    comment = probes[1].get('format', {}).get('tags', {}).get('comment', '')
    if 'Gyroflow export LUT applied:' not in comment or 'Gyroflow color adjustments:' not in comment:
        raise RuntimeError('Colored export is missing its LUT/adjustment metadata markers.')
    report = dict(decoded_frames=args.expected_frames, all_frame_hashes_and_timestamps_equal=True,
                  full_decode_passed=True, stream=probes[1]['streams'][0],
                  lut_metadata_marker=True, adjustment_metadata_marker=True,
                  brightness=args.brightness, contrast=args.contrast)
    (output / 'result.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
