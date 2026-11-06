// Exercise the actual QML validation functions with controller/UI boundary doubles.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { qmlMethod } from './helpers/qml-method.mjs';
import assert from 'node:assert/strict';
const source = readFileSync(new URL('../src/ui/menu/LensCalibrate.qml', import.meta.url), 'utf8');
const method = name => qmlMethod(source, name);
function validate({brand='Sony', model='New model', lens='24–70mm', focal=35, modelIndex=0, lensIndex=0, upload=true, setting='', cameraInfo={}, selectedBrand=brand}={}) {
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
        list: {commitAll() {}}, cameraBrand:{currentIndex:1,model:['Other',selectedBrand]},
        cameraModel:{currentIndex:modelIndex,model:['Other','Alpha 7 III']}, cameraLens:{currentIndex:lensIndex,model:['Other','24–70mm']},
        upload, flcb:{checked:focal>0}, qsTr:s=>s, Modal:{Error:1}, messageBox() {}
    });
    return vm.runInContext(['cameraNeedsLens','selectorValue','isZoomLens','hasSetupName','validateProfileMetadata'].map(method).join('\n')+'\nvalidateProfileMetadata(upload)',context);
}
assert.equal(validate(), true, 'known brand with Other model is allowed');
assert.equal(validate({modelIndex:1}), false, 'manual unknown camera must use Other');
assert.equal(validate({lens:'', setting:'4K'}), false, 'video settings do not identify an interchangeable lens');
for (const lens of ['24-70mm','24–70mm','24—70mm','Power Zoom','18-105 F4','16-35','24mm–70mm']) {
    assert.equal(validate({lens,focal:0}), false, `${lens} needs focal length`);
}
assert.equal(validate({lens:'24',lensIndex:1}), false, 'partial text cannot impersonate a catalog lens');
assert.equal(validate({brand:'Canon',selectedBrand:'Sony'}), false, 'metadata must agree with a selected brand');
assert.equal(validate({model:'Alpha 7 III',modelIndex:1,lens:'24–70mm',lensIndex:1}), true, 'selected setup is accepted');
assert.equal(validate({focal:Infinity}), false, 'zoom focal length must be finite');
assert.equal(validate({lens:'35mm F1.8',focal:0}), true, 'a prime does not require zoom focal metadata');
assert.equal(validate({brand:'Unknown',model:'Other',lens:'---'}), false, 'placeholders cannot identify an upload');
assert.equal(validate({lens:'N/A'}), false, 'a placeholder is not an interchangeable lens');
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
