// Exercise the actual QML validation functions with controller/UI boundary doubles.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source = readFileSync(new URL('../src/ui/menu/LensCalibrate.qml', import.meta.url), 'utf8');
function method(name) {
    const start = source.indexOf(`    function ${name}(`);
    const end = source.indexOf('\n    }', start) + 6;
    return source.slice(start, end).replace(/: (string|bool|var|void)(?=[,)\s{])/g, '');
}
function validate({brand='Sony', model='New model', lens='24–70mm', focal=35, modelIndex=0, lensIndex=0, upload=true, setting='', cameraInfo={}}={}) {
    const context = vm.createContext({
        calib: {
            calibrationInfo: {camera_brand:brand, camera_model:model, lens_model:lens, focal_length:focal, camera_setting:setting},
            uploadProfile:{checked:upload,enabled:true}
        },
        controller: {
            camera_database_info: () => cameraInfo,
            camera_database_resolve_model: () => '',
            camera_database_lenses: () => ['24–70mm']
        },
        list: {commitAll() {}}, cameraBrand:{currentIndex:1},
        cameraModel:{currentIndex:modelIndex}, cameraLens:{currentIndex:lensIndex},
        flcb:{checked:focal>0}, qsTr:s=>s, Modal:{Error:1}, messageBox() {}
    });
    return vm.runInContext(['cameraNeedsLens','cameraIsKnown','lensIsKnown','validateProfileMetadata'].map(method).join('\n')+'\nvalidateProfileMetadata()',context);
}
assert.equal(validate(), true, 'known brand with Other model is allowed');
assert.equal(validate({modelIndex:1}), false, 'manual unknown camera must use Other');
assert.equal(validate({lens:'', setting:'4K'}), false, 'video settings do not identify an interchangeable lens');
for (const lens of ['24-70mm','24–70mm','24—70mm','Power Zoom']) {
    assert.equal(validate({lens,focal:0}), false, `${lens} needs focal length`);
}
assert.equal(validate({lens:'24',lensIndex:1}), false, 'partial text cannot impersonate a catalog lens');
assert.equal(validate({brand:'',model:'',lens:'',upload:false}), true, 'local export remains available');
function needsLens(brand, model, cameraInfo={}) {
    const context = vm.createContext({controller:{camera_database_info:()=>cameraInfo}});
    return vm.runInContext(method('cameraNeedsLens')+'\ncameraNeedsLens('+JSON.stringify(brand)+','+JSON.stringify(model)+')',context);
}
assert.equal(needsLens('GoPro','HERO4 Black',{mounts:['goProHero4black']}), false, 'Lensfun lower-case mount identifies a fixed-lens camera');
assert.equal(needsLens('Canon','PowerShot G7 X',{mounts:['canonG7X']}), false, 'Lensfun lower-case mount identifies a fixed-lens compact');
assert.equal(needsLens('Sony','ILCE-7',{mounts:['E-mount']}), true, 'an uppercase interchangeable mount requires lens identification');
assert.equal(needsLens('DJI','Zenmuse X7'), true, 'Zenmuse cameras are not classified from the DJI brand alone');
assert.equal(needsLens('DJI','Osmo Pocket'), false, 'known fixed-lens DJI models do not require an interchangeable lens');
assert.equal(validate({brand:'DJI',model:'Future camera',lens:'',modelIndex:0}), false, 'unknown DJI cameras selected through Other still require lens identification');
assert.equal(validate({brand:'DJI',model:'Future camera',lens:'24mm',modelIndex:1}), false, 'unknown DJI camera text requires explicit Other selection');
console.log('Calibration metadata regressions passed');
