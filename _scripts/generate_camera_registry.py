#!/usr/bin/env python3
"""
Generate camera registry JSON from:
1. Existing Gyroflow lens profiles
2. LensFun database XML files

This script extracts unique camera brands, models, and lenses to create
a standardized registry for UI selectors in Gyroflow.

Usage:
    python generate_camera_registry.py [--profiles-dir PATH] [--lensfun-dir PATH] [--output PATH]

Output:
    camera_registry.json - The canonical camera/lens database
"""

import json
import os
import sys
import re
import xml.etree.ElementTree as ET
from collections import defaultdict
from pathlib import Path
from typing import Dict, List, Set, Tuple, Optional
import argparse
import gzip

BRAND_NORMALIZATION = {
    'gopro': 'GoPro',
    'go pro': 'GoPro',
    'dji': 'DJI',
    'sony': 'Sony',
    'ilce': 'Sony',
    'ilme': 'Sony',
    'canon': 'Canon',
    'nikon': 'Nikon',
    'panasonic': 'Panasonic',
    'lumix': 'Panasonic',
    'fujifilm': 'Fujifilm',
    'fuji': 'Fujifilm',
    'blackmagic': 'Blackmagic',
    'blackmagic design': 'Blackmagic',
    'bmpcc': 'Blackmagic',
    'insta360': 'Insta360',
    'red': 'RED',
    'red digital cinema': 'RED',
    'apple': 'Apple',
    'samsung': 'Samsung',
    'olympus': 'Olympus',
    'om system': 'Olympus',
    'om-system': 'Olympus',
    'leica': 'Leica',
    'hasselblad': 'Hasselblad',
    'runcam': 'RunCam',
    'caddx': 'Caddx',
    'arri': 'ARRI',
    'sigma': 'Sigma',
    'tamron': 'Tamron',
    'zeiss': 'Zeiss',
    'samyang': 'Samyang',
    'rokinon': 'Samyang',
    'voigtlander': 'Voigtlander',
    'tokina': 'Tokina',
    'laowa': 'Laowa',
    'venus optics': 'Laowa',
}

BRAND_ALIASES = {
    'Sony': ['ILCE', 'ILME', 'α', 'Alpha'],
    'Canon': ['EOS'],
    'Nikon': [],
    'Panasonic': ['Lumix', 'DC-', 'DMC-'],
    'Fujifilm': ['Fuji', 'X-'],
    'Olympus': ['OM System', 'OM-D', 'PEN'],
    'GoPro': ['Hero', 'HERO'],
    'DJI': ['Osmo', 'Mavic', 'Air', 'Mini', 'Phantom', 'Inspire'],
    'Blackmagic': ['BMPCC', 'BMCC', 'BRAW'],
    'Insta360': ['ONE', 'GO'],
    'RED': ['DSMC', 'KOMODO', 'Scarlet', 'Epic', 'Monstro', 'Helium'],
    'Apple': ['iPhone', 'iPad'],
    'Samsung': ['Galaxy'],
}

MOUNTS = {
    'Sony E': {'compatible': [], 'flange_mm': 18.0},
    'Sony A': {'compatible': ['Sony E'], 'flange_mm': 44.5},
    'Canon EF': {'compatible': [], 'flange_mm': 44.0},
    'Canon EF-S': {'compatible': ['Canon EF'], 'flange_mm': 44.0},
    'Canon RF': {'compatible': ['Canon EF', 'Canon EF-S'], 'flange_mm': 20.0},
    'Canon EF-M': {'compatible': ['Canon EF', 'Canon EF-S'], 'flange_mm': 18.0},
    'Nikon F': {'compatible': [], 'flange_mm': 46.5},
    'Nikon Z': {'compatible': ['Nikon F'], 'flange_mm': 16.0},
    'Nikon 1': {'compatible': [], 'flange_mm': 17.0},
    'Micro Four Thirds': {'compatible': [], 'flange_mm': 19.25},
    'Fujifilm X': {'compatible': [], 'flange_mm': 17.7},
    'Fujifilm GFX': {'compatible': [], 'flange_mm': 26.7},
    'Leica L': {'compatible': [], 'flange_mm': 20.0},
    'Leica M': {'compatible': [], 'flange_mm': 27.8},
    'Leica SL': {'compatible': ['Leica L'], 'flange_mm': 20.0},
    'Pentax K': {'compatible': [], 'flange_mm': 45.46},
    'Samsung NX': {'compatible': [], 'flange_mm': 25.5},
    'ARRI PL': {'compatible': [], 'flange_mm': 52.0},
    'C mount': {'compatible': [], 'flange_mm': 17.526},
}

SENSOR_SIZES = {
    'Full Frame': {'width': 36.0, 'height': 24.0, 'crop': 1.0},
    'APS-C Sony': {'width': 23.5, 'height': 15.6, 'crop': 1.5},
    'APS-C Canon': {'width': 22.3, 'height': 14.9, 'crop': 1.6},
    'APS-C Nikon': {'width': 23.5, 'height': 15.6, 'crop': 1.5},
    'APS-C Fuji': {'width': 23.5, 'height': 15.6, 'crop': 1.5},
    'Micro Four Thirds': {'width': 17.3, 'height': 13.0, 'crop': 2.0},
    'Medium Format': {'width': 43.8, 'height': 32.9, 'crop': 0.79},
    '1 inch': {'width': 13.2, 'height': 8.8, 'crop': 2.7},
    '1/2.3 inch': {'width': 6.17, 'height': 4.55, 'crop': 5.64},
    'Super 35': {'width': 24.89, 'height': 18.66, 'crop': 1.39},
}


def normalize_brand(brand: str) -> str:
    """Normalize brand name to canonical form."""
    brand_lower = brand.lower().strip()
    return BRAND_NORMALIZATION.get(brand_lower, brand.strip())


def infer_mount(brand: str, model: str) -> str:
    """Infer camera mount from brand and model."""
    model_lower = model.lower()
    
    if brand == 'Sony':
        if any(x in model_lower for x in ['a7', 'a9', 'fx3', 'fx6', 'fx9', 'a1']):
            return 'Sony E'
        elif any(x in model_lower for x in ['a6', 'zv-e', 'nex', 'a5']):
            return 'Sony E'
        elif any(x in model_lower for x in ['a99', 'a77', 'a65', 'a55']):
            return 'Sony A'
        return 'Sony E'
    
    elif brand == 'Canon':
        if any(x in model_lower for x in ['eos r', 'r3', 'r5', 'r6', 'r7', 'r8', 'r10', 'r50', 'r100']):
            return 'Canon RF'
        elif any(x in model_lower for x in ['eos m', 'm50', 'm6', 'm5', 'm3', 'm200']):
            return 'Canon EF-M'
        return 'Canon EF'
    
    elif brand == 'Nikon':
        if model_lower.startswith('z') or any(x in model_lower for x in ['z5', 'z6', 'z7', 'z8', 'z9', 'z30', 'z50', 'zf', 'zfc']):
            return 'Nikon Z'
        elif 'nikon 1' in model_lower or model_lower.startswith('j') or model_lower.startswith('v'):
            return 'Nikon 1'
        return 'Nikon F'
    
    elif brand == 'Panasonic':
        if any(x in model_lower for x in ['s1', 's5', 's9']):
            return 'Leica L'
        return 'Micro Four Thirds'
    
    elif brand == 'Olympus':
        return 'Micro Four Thirds'
    
    elif brand == 'Fujifilm':
        if 'gfx' in model_lower:
            return 'Fujifilm GFX'
        return 'Fujifilm X'
    
    elif brand == 'Leica':
        if any(x in model_lower for x in ['sl', 'fp', 'cl', 'tl']):
            return 'Leica L'
        return 'Leica M'
    
    elif brand == 'ARRI':
        return 'ARRI PL'
    
    return ''


def infer_crop_factor(brand: str, model: str) -> Optional[float]:
    """Infer crop factor from brand and model."""
    model_lower = model.lower()
    
    if brand == 'Sony':
        if any(x in model_lower for x in ['a7', 'a9', 'a1', 'fx3', 'fx6', 'fx9']):
            return 1.0
        elif any(x in model_lower for x in ['a6', 'zv-e', 'nex']):
            return 1.5
    
    elif brand == 'Canon':
        if any(x in model_lower for x in ['5d', '6d', 'r3', 'r5', 'r6', 'r8', '1d']):
            return 1.0
        elif any(x in model_lower for x in ['7d', 'r7', 'r10', 'r50', 'r100', '90d', '80d', 'm50', 'm6']):
            return 1.6
    
    elif brand == 'Nikon':
        if any(x in model_lower for x in ['d8', 'd7', 'd6', 'z5', 'z6', 'z7', 'z8', 'z9', 'zf']):
            return 1.0
        elif any(x in model_lower for x in ['d5', 'd3', 'z30', 'z50', 'zfc']):
            return 1.5
    
    elif brand in ['Panasonic', 'Olympus']:
        if any(x in model_lower for x in ['s1', 's5', 's9']):
            return 1.0
        return 2.0
    
    elif brand == 'Fujifilm':
        if 'gfx' in model_lower:
            return 0.79
        return 1.5
    
    elif brand == 'GoPro':
        return 1.0
    
    elif brand == 'DJI':
        if any(x in model_lower for x in ['mavic 3', 'inspire']):
            return 1.0
    
    elif brand == 'RED':
        if 'monstro' in model_lower or 'helium' in model_lower:
            return 1.0
        elif 'komodo' in model_lower:
            return 1.5
    
    return None


def is_fixed_lens_camera(brand: str, model: str) -> bool:
    """Check if camera has a fixed lens (not interchangeable)."""
    model_lower = model.lower()
    
    fixed_lens_brands = ['GoPro', 'DJI', 'Insta360', 'RunCam', 'Caddx', 'Apple']
    if brand in fixed_lens_brands:
        return True
    
    if brand == 'Sony':
        if any(x in model_lower for x in ['rx100', 'rx10', 'rx1', 'zv-1', 'hx', 'wx']):
            return True
    
    if brand == 'Canon':
        if any(x in model_lower for x in ['powershot', 'g7x', 'g5x', 'g9x', 'sx']):
            return True
    
    if brand == 'Panasonic':
        if any(x in model_lower for x in ['lx100', 'lx15', 'fz', 'tz', 'zs']):
            return True
    
    if brand == 'Fujifilm':
        if any(x in model_lower for x in ['x100', 'x70', 'xf10', 'xq']):
            return True
    
    return False


def is_zoom_lens(lens_name: str, min_focal: Optional[float] = None, max_focal: Optional[float] = None) -> bool:
    """Determine if a lens is a zoom lens."""
    if min_focal is not None and max_focal is not None:
        return abs(max_focal - min_focal) > 0.1
    
    lens_lower = lens_name.lower()
    if '-' in lens_lower and 'mm' in lens_lower:
        match = re.search(r'(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*mm', lens_lower)
        if match:
            return float(match.group(1)) != float(match.group(2))
    
    return False


def parse_gyroflow_profiles(profiles_dir: Path) -> Tuple[Dict, List[Tuple]]:
    """Parse Gyroflow lens profile JSON files."""
    profiles_data = defaultdict(lambda: {
        'cameras': defaultdict(lambda: {'aliases': set(), 'mount': '', 'crop': None, 'fixed_lens': False}),
        'lenses': defaultdict(lambda: {'aliases': set(), 'mounts': set(), 'is_zoom': False}),
    })
    
    raw_profiles = []
    
    def process_file(file_path: Path, content: str):
        try:
            profile = json.loads(content)
            brand = normalize_brand(profile.get('camera_brand', ''))
            model = profile.get('camera_model', '').strip()
            lens = profile.get('lens_model', '').strip()
            setting = profile.get('camera_setting', '').strip()
            
            if not brand or not model:
                return
            
            raw_profiles.append((brand, model, lens, setting))
            
            mount = infer_mount(brand, model)
            crop = infer_crop_factor(brand, model)
            fixed = is_fixed_lens_camera(brand, model)
            
            profiles_data[brand]['cameras'][model]['mount'] = mount
            if crop:
                profiles_data[brand]['cameras'][model]['crop'] = crop
            profiles_data[brand]['cameras'][model]['fixed_lens'] = fixed
            
            if lens and not fixed:
                zoom = is_zoom_lens(lens)
                profiles_data[brand]['lenses'][lens]['is_zoom'] = zoom
                if mount:
                    profiles_data[brand]['lenses'][lens]['mounts'].add(mount)
                    
        except (json.JSONDecodeError, KeyError) as e:
            pass
    
    if not profiles_dir.exists():
        return profiles_data, raw_profiles
    
    cbor_file = profiles_dir / 'profiles.cbor.gz'
    if cbor_file.exists():
        try:
            import cbor2
            with gzip.open(cbor_file, 'rb') as f:
                data = cbor2.load(f)
                for name, profile in data:
                    if name.startswith('__'):
                        continue
                    if isinstance(profile, dict):
                        process_file(Path(name), json.dumps(profile))
        except Exception as e:
            print(f"Warning: Could not parse CBOR file: {e}", file=sys.stderr)
    
    for json_file in profiles_dir.rglob('*.json'):
        try:
            content = json_file.read_text(encoding='utf-8')
            process_file(json_file, content)
        except Exception as e:
            pass
    
    return profiles_data, raw_profiles


def parse_lensfun_xml(lensfun_dir: Path) -> Dict:
    """Parse LensFun XML database files."""
    lensfun_data = {
        'mounts': {},
        'cameras': defaultdict(lambda: defaultdict(lambda: {'aliases': set(), 'mount': '', 'crop': None})),
        'lenses': []
    }
    
    if not lensfun_dir.exists():
        return lensfun_data
    
    for xml_file in lensfun_dir.rglob('*.xml'):
        try:
            tree = ET.parse(xml_file)
            root = tree.getroot()
            
            for mount in root.findall('.//mount'):
                name = mount.find('name')
                if name is not None and name.text:
                    compat = [c.text for c in mount.findall('compat') if c.text]
                    lensfun_data['mounts'][name.text] = {'compatible': compat}
            
            for camera in root.findall('.//camera'):
                maker = camera.find('maker')
                model = camera.find('model')
                mount_elem = camera.find('mount')
                cropfactor = camera.find('cropfactor')
                
                if maker is not None and model is not None and maker.text and model.text:
                    brand = normalize_brand(maker.text)
                    model_name = model.text.strip()
                    
                    lensfun_data['cameras'][brand][model_name]['from_lensfun'] = True
                    
                    if mount_elem is not None and mount_elem.text:
                        lensfun_data['cameras'][brand][model_name]['mount'] = mount_elem.text
                    
                    if cropfactor is not None and cropfactor.text:
                        try:
                            lensfun_data['cameras'][brand][model_name]['crop'] = float(cropfactor.text)
                        except ValueError:
                            pass
            
            for lens in root.findall('.//lens'):
                maker = lens.find('maker')
                model = lens.find('model')
                
                if maker is not None and model is not None and maker.text and model.text:
                    lens_info = {
                        'maker': normalize_brand(maker.text),
                        'model': model.text.strip(),
                        'mounts': [m.text for m in lens.findall('mount') if m.text],
                        'min_focal': None,
                        'max_focal': None,
                    }
                    
                    focal = lens.find('focal')
                    if focal is not None:
                        if focal.get('min'):
                            try:
                                lens_info['min_focal'] = float(focal.get('min'))
                            except ValueError:
                                pass
                        if focal.get('max'):
                            try:
                                lens_info['max_focal'] = float(focal.get('max'))
                            except ValueError:
                                pass
                    
                    lensfun_data['lenses'].append(lens_info)
                    
        except ET.ParseError as e:
            print(f"Warning: Could not parse {xml_file}: {e}", file=sys.stderr)
    
    return lensfun_data


def merge_data(gyroflow_data: Dict, lensfun_data: Dict) -> Dict:
    """Merge Gyroflow and LensFun data into unified registry."""
    registry = {
        'version': 1,
        'brands': [],
        'mounts': [],
        'lensfun_lenses': []
    }
    
    for name, info in MOUNTS.items():
        mount_entry = {
            'name': name,
            'compatible_mounts': info['compatible'],
        }
        if info.get('flange_mm'):
            mount_entry['flange_distance_mm'] = info['flange_mm']
        registry['mounts'].append(mount_entry)
    
    for name, info in lensfun_data.get('mounts', {}).items():
        existing = next((m for m in registry['mounts'] if m['name'].lower() == name.lower()), None)
        if not existing:
            registry['mounts'].append({
                'name': name,
                'compatible_mounts': info.get('compatible', []),
            })
    
    all_brands = set(gyroflow_data.keys()) | set(lensfun_data.get('cameras', {}).keys())
    
    for brand in sorted(all_brands):
        brand_entry = {
            'name': brand,
            'aliases': BRAND_ALIASES.get(brand, []),
            'cameras': [],
            'lenses': []
        }
        
        cameras = {}
        if brand in gyroflow_data:
            for model, info in gyroflow_data[brand]['cameras'].items():
                cameras[model] = {
                    'name': model,
                    'aliases': list(info.get('aliases', set())),
                    'mount': info.get('mount', ''),
                    'crop_factor': info.get('crop'),
                    'fixed_lens': info.get('fixed_lens', False),
                    'from_lensfun': False
                }
        
        if brand in lensfun_data.get('cameras', {}):
            for model, info in lensfun_data['cameras'][brand].items():
                if model in cameras:
                    if not cameras[model]['mount'] and info.get('mount'):
                        cameras[model]['mount'] = info['mount']
                    if not cameras[model]['crop_factor'] and info.get('crop'):
                        cameras[model]['crop_factor'] = info['crop']
                else:
                    cameras[model] = {
                        'name': model,
                        'aliases': list(info.get('aliases', set())),
                        'mount': info.get('mount', ''),
                        'crop_factor': info.get('crop'),
                        'fixed_lens': False,
                        'from_lensfun': True
                    }
        
        for model, info in sorted(cameras.items()):
            camera_entry = {'name': info['name']}
            if info['aliases']:
                camera_entry['aliases'] = info['aliases']
            if info['mount']:
                camera_entry['mount'] = info['mount']
            if info['crop_factor']:
                camera_entry['crop_factor'] = info['crop_factor']
            if info['fixed_lens']:
                camera_entry['fixed_lens'] = True
            if info['from_lensfun']:
                camera_entry['from_lensfun'] = True
            brand_entry['cameras'].append(camera_entry)
        
        if brand in gyroflow_data:
            lenses = {}
            for lens_name, info in gyroflow_data[brand]['lenses'].items():
                lenses[lens_name] = {
                    'name': lens_name,
                    'aliases': list(info.get('aliases', set())),
                    'mounts': list(info.get('mounts', set())),
                    'is_zoom': info.get('is_zoom', False)
                }
            
            for lens_name, info in sorted(lenses.items()):
                lens_entry = {'name': info['name']}
                if info['aliases']:
                    lens_entry['aliases'] = info['aliases']
                if info['mounts']:
                    lens_entry['mounts'] = info['mounts']
                if info['is_zoom']:
                    lens_entry['is_zoom'] = True
                brand_entry['lenses'].append(lens_entry)
        
        registry['brands'].append(brand_entry)
    
    for lens in lensfun_data.get('lenses', []):
        registry['lensfun_lenses'].append({
            'maker': lens['maker'],
            'model': lens['model'],
            'mounts': lens['mounts'],
            'min_focal': lens['min_focal'],
            'max_focal': lens['max_focal']
        })
    
    return registry


def main():
    parser = argparse.ArgumentParser(description='Generate camera registry JSON')
    parser.add_argument('--profiles-dir', type=Path, default=Path('resources/camera_presets'),
                        help='Directory containing Gyroflow lens profiles')
    parser.add_argument('--lensfun-dir', type=Path, default=Path('/usr/share/lensfun/version_2'),
                        help='Directory containing LensFun XML files')
    parser.add_argument('--output', type=Path, default=Path('resources/camera_registry.json'),
                        help='Output JSON file path')
    parser.add_argument('--pretty', action='store_true', help='Pretty-print JSON output')
    
    args = parser.parse_args()
    
    print(f"Parsing Gyroflow profiles from {args.profiles_dir}...", file=sys.stderr)
    gyroflow_data, raw_profiles = parse_gyroflow_profiles(args.profiles_dir)
    print(f"  Found {sum(len(b['cameras']) for b in gyroflow_data.values())} cameras", file=sys.stderr)
    print(f"  Found {sum(len(b['lenses']) for b in gyroflow_data.values())} lenses", file=sys.stderr)
    
    print(f"Parsing LensFun database from {args.lensfun_dir}...", file=sys.stderr)
    lensfun_data = parse_lensfun_xml(args.lensfun_dir)
    print(f"  Found {sum(len(c) for c in lensfun_data['cameras'].values())} cameras", file=sys.stderr)
    print(f"  Found {len(lensfun_data['lenses'])} lenses", file=sys.stderr)
    
    print("Merging data...", file=sys.stderr)
    registry = merge_data(gyroflow_data, lensfun_data)
    
    args.output.parent.mkdir(parents=True, exist_ok=True)
    
    indent = 2 if args.pretty else None
    with open(args.output, 'w', encoding='utf-8') as f:
        json.dump(registry, f, indent=indent, ensure_ascii=False)
    
    print(f"Wrote camera registry to {args.output}", file=sys.stderr)
    print(f"  Total brands: {len(registry['brands'])}", file=sys.stderr)
    print(f"  Total cameras: {sum(len(b['cameras']) for b in registry['brands'])}", file=sys.stderr)
    print(f"  Total lenses: {sum(len(b['lenses']) for b in registry['brands'])}", file=sys.stderr)
    print(f"  LensFun lenses: {len(registry['lensfun_lenses'])}", file=sys.stderr)


if __name__ == '__main__':
    main()
