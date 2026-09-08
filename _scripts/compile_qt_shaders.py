"""Compile Gyroflow's documented Qt shader variants with a supplied qsb binary."""
from pathlib import Path
import argparse
import json
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--root', required=True)
parser.add_argument('--qsb', required=True)
args = parser.parse_args()
root = Path(args.root).resolve()
out = root / 'src/qt_gpu/compiled'
template = (root / 'src/qt_gpu/undistort.frag').read_text()
models = ['opencv_fisheye', 'opencv_standard', 'poly3', 'poly5', 'ptlens', 'insta360', 'sony', 'generic_polynomial', 'gopro']
digital = ['', 'gopro_superview', 'gopro6_superview', 'gopro_hyperview', 'gopro_warp', 'digital_stretch']
options = [args.qsb, '--glsl', '120,300 es,310 es,320 es,310,320,330,400,410,420', '--hlsl', '50', '--msl', '12']
compiled = []
with tempfile.TemporaryDirectory(prefix='gyroflow-shaders-') as temporary:
    for model in models:
        for lens in digital:
            if lens in ['gopro_superview', 'gopro6_superview', 'gopro_hyperview'] and model != 'opencv_fisheye':
                continue
            if lens == 'gopro_warp' and model != 'gopro':
                continue
            if model == 'gopro' and lens not in ['', 'gopro_warp']:
                continue
            functions = (root / f'src/core/stabilization/distortion_models/{lens}.glsl').read_text() if lens else 'vec2 digital_undistort_point(vec2 uv) { return uv; } vec2 digital_distort_point(vec2 uv) { return uv; }'
            if model not in ['sony', 'generic_polynomial']:
                functions += '\nvec2 process_coord(vec2 uv, float idx) { return uv; }'
            functions += '\n' + (root / f'src/core/stabilization/distortion_models/{model}.glsl').read_text()
            source = template.replace('LENS_MODEL_FUNCTIONS;', functions)
            if model in ['sony', 'generic_polynomial']:
                source += '\nfloat get_mesh_data(int idx) { return texture(texMeshData, vec2(0, idx / 2047.0)).r; }\n'
            target = out / f'undistort_{model}{"_" + lens if lens else ""}.frag.qsb'
            fragment = Path(temporary) / 'shader.frag'
            fragment.write_text(source)
            subprocess.run(options + ['-o', str(target), str(fragment)], check=True)
            compiled.append(target.name)
print(json.dumps({'count': len(compiled), 'compiled': compiled}, indent=2))
