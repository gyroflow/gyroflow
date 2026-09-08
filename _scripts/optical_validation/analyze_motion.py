# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare real-footage motion with an independent, fixed central phase-correlation metric."""
from pathlib import Path
import argparse, json, subprocess
import numpy as np

parser=argparse.ArgumentParser()
parser.add_argument('--ffmpeg',required=True)
parser.add_argument('--directory',required=True)
parser.add_argument('--video',action='append',required=True,help='label=path')
parser.add_argument('--frames',type=int,required=True)
parser.add_argument('--output',default='real-motion-metrics.json')
args=parser.parse_args()
root=Path(args.directory).resolve()
w,h=640,360
report={'analysis_size':[w,h],'frames':args.frames,'results':{},
    'scope':'Fixed central phase correlation at 640x360, every adjacent frame pair retained. On real footage there is no ground-truth camera path; rotation, zoom, parallax and scene motion can affect this proxy. Use alongside visual review, crop measurements and local-motion evaluation.'}
for entry in args.video:
    label,name=entry.split('=',1)
    decoded=subprocess.run([args.ffmpeg,'-v','error','-i',str(root/name),'-vf',f'scale={w}:{h}',
        '-pix_fmt','gray','-f','rawvideo','pipe:1'],capture_output=True,check=True)
    frames=np.frombuffer(decoded.stdout,dtype=np.uint8).reshape(-1,h,w)
    if len(frames)!=args.frames:
        raise ValueError(f'{label}: decoded {len(frames)}, expected {args.frames}')
    frame_crop=frames[:,h//6:h-h//6,w//6:w-w//6]
    ch,cw=frame_crop.shape[1:]
    window=np.outer(np.hanning(ch),np.hanning(cw))
    shifts=[];peaks=[];previous=None
    for frame in frame_crop:
        spectrum=np.fft.rfft2((frame.astype(float)-frame.mean())*window)
        if previous is not None:
            cross=previous*np.conj(spectrum)
            cross/=np.maximum(np.abs(cross),1e-12)
            corr=np.fft.irfft2(cross,s=(ch,cw))
            y,x=np.unravel_index(np.argmax(corr),corr.shape)
            def subpixel(a,b,c):
                denom=a-2*b+c
                return float(np.clip(0.5*(a-c)/denom,-0.5,0.5)) if abs(denom)>1e-12 else 0.0
            dx=subpixel(corr[y,(x-1)%cw],corr[y,x],corr[y,(x+1)%cw])
            dy=subpixel(corr[(y-1)%ch,x],corr[y,x],corr[(y+1)%ch,x])
            shifts.append([-(int(x) if x<=cw//2 else int(x)-cw)-dx,-(int(y) if y<=ch//2 else int(y)-ch)-dy])
            peaks.append(float(corr[y,x]))
        previous=spectrum
    shifts=np.asarray(shifts)
    acceleration=np.diff(shifts,axis=0)
    report['results'][label]={'pairs':len(shifts),
        'translation_acceleration_rms_px':float(np.sqrt(np.mean(np.sum(acceleration**2,axis=1)))),
        'translation_rms_px_per_frame':float(np.sqrt(np.mean(np.sum(shifts**2,axis=1)))),
        'total_translation_px':shifts.sum(axis=0).tolist(),
        'minimum_correlation_peak':min(peaks),'median_correlation_peak':float(np.median(peaks)),
        'fraction_peaks_below_0_1':float(np.mean(np.asarray(peaks)<0.1)),
        'shifts_px':shifts.tolist(),'correlation_peaks':peaks}
if {'control','residual'}.issubset(report['results']):
    report['acceleration_proxy_reduction_percent']=100*(1-report['results']['residual']['translation_acceleration_rms_px']/report['results']['control']['translation_acceleration_rms_px'])
(root/args.output).write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({**report,'results':{k:{a:b for a,b in v.items() if a not in ['shifts_px','correlation_peaks']} for k,v in report['results'].items()}},indent=2))
