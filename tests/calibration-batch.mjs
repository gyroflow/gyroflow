// Exercise batch result handling and the real calibration export helper with async boundary doubles.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { qmlMethod as method } from './helpers/qml-method.mjs';
import assert from 'node:assert/strict';

const calibratorSource = readFileSync(new URL('../src/ui/Calibrator.qml', import.meta.url), 'utf8');
const lensCalibrateSource = readFileSync(new URL('../src/ui/menu/LensCalibrate.qml', import.meta.url), 'utf8');
const submissionModule = vm.createContext({});
vm.runInContext(readFileSync(new URL('../src/ui/menu/ProfileSubmission.js', import.meta.url), 'utf8'), submissionModule);


function harness({upload=true, valid=true}={}) {
    const scheduled = [];
    const exports = [];
    const lensCalib = {
        calibrationInfo:{camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:35},
        uploadProfile:{checked:upload},
        valid,
        validationCalls:[],
        prepareProfileExport:null,
        savePreparedProfile:null,
        validateProfileMetadata(requested) {
            this.validationCalls.push(requested);
            return !requested || this.valid;
        }
    };
    const state = {
        ProfileSubmission: submissionModule,
        batch:{
            active:true,finalizing:true,currentFile:'clip.mp4',
            runIn(ms,cb) { scheduled.push({ms,cb}); },
            startCalls:0,
            start() { this.startCalls++; }
        },
        lensCalib,calib:lensCalib,
        controller:{
            export_lens_profile_filename(metadata) { return `${metadata.camera_model}.json`; },
            export_lens_profile(url,metadata,upload) {
                exports.push({url,metadata:structuredClone(metadata),upload});
                return true;
            }
        },
        calibrator_window:{
            getSaveFileUrl(folder,filename,onAccepted,type,onCancel) {
                state.picker={folder,filename,onAccepted,onCancel,type};
            },
            showNotification() {}
        },
        filesystem:{
            get_folder(file) { return `folder:${file}`; },
            get_filename(file) { return String(file).split('/').pop(); },
            filename_with_extension(file,extension) { return `${file.replace(/\.[^.]+$/,'')}.${extension}`; },
            exists_in_folder() { return false; },
            display_url(url) { return url; }
        },
        Modal:{Info:1},
        qsTr:text=>Object.assign(new String(text),{arg:value=>text.replace('%1',value)}),
        messageBox(type,text,buttons) { state.consent={type,text,buttons}; },
        state:null,
        picker:null,consent:null
    };
    state.state=state;
    const context=vm.createContext(state);
    vm.runInContext([
        method(lensCalibrateSource,'prepareProfileExport','    '),
        method(lensCalibrateSource,'savePreparedProfile','    '),
        method(calibratorSource,'completeCurrent','        '),
        method(calibratorSource,'saveResult','        ')
    ].join('\n'),context);
    lensCalib.prepareProfileExport=(...args)=>context.prepareProfileExport(...args);
    lensCalib.savePreparedProfile=(...args)=>context.savePreparedProfile(...args);
    state.batch.completeCurrent=()=>context.completeCurrent();
    state.batch.saveResult=(...args)=>context.saveResult(...args);
    return {state,context,scheduled,exports,lensCalib};
}

function advance(state, scheduled) {
    assert.equal(scheduled.length,1,'batch schedules exactly one next-file transition');
    scheduled.shift().cb();
    assert.equal(state.batch.startCalls,1,'batch resumes once');
    assert.equal(state.batch.finalizing,false,'next clip can finalize');
}

{
    const h=harness({upload:true,valid:false});
    h.state.batch.saveResult(0.5);
    assert.deepEqual(h.lensCalib.validationCalls,[true],'batch validates requested upload before destination selection');
    assert.equal(h.state.picker, null,'invalid upload metadata never opens a destination picker');
    assert.equal(h.exports.length,0,'invalid upload metadata is never exported');
    advance(h.state,h.scheduled);
}

{
    const h=harness({upload:true,valid:true});
    const original=structuredClone(h.lensCalib.calibrationInfo);
    h.state.batch.saveResult(0.5);
    assert.ok(h.state.picker,'valid calibration opens a destination picker');
    assert.equal(h.scheduled.length,0,'batch waits for destination selection');
    h.lensCalib.calibrationInfo.camera_model='next clip';
    h.state.picker.onAccepted('file:///first-profile.json');
    assert.deepEqual(Array.from(h.state.consent.buttons,button=>String(button.text)),['Yes','No','Cancel']);
    assert.equal(h.exports.length,0,'batch waits for explicit upload consent');
    assert.equal(h.scheduled.length,0,'next clip does not start while consent is pending');
    h.state.consent.buttons[0].clicked();
    assert.deepEqual(h.exports[0],{url:'file:///first-profile.json',metadata:original,upload:true},'consent callback exports the validated first-clip snapshot');
    advance(h.state,h.scheduled);
}

{
    const h=harness({upload:true,valid:true});
    h.state.batch.saveResult(0.5);
    h.state.picker.onCancel();
    assert.equal(h.exports.length,0,'canceled destination never exports');
    advance(h.state,h.scheduled);
}

{
    const h=harness({upload:true,valid:true});
    h.state.batch.saveResult(0.5);
    h.state.picker.onAccepted('file:///profile.json');
    h.state.consent.buttons[2].clicked();
    assert.equal(h.exports.length,0,'canceled consent never exports locally or remotely');
    advance(h.state,h.scheduled);
}

{
    const h=harness({upload:false,valid:false});
    h.state.batch.saveResult(0.5);
    assert.deepEqual(h.lensCalib.validationCalls,[false],'local batch export does not require upload metadata');
    h.state.picker.onAccepted('file:///local-profile.json');
    assert.deepEqual(h.exports[0],{
        url:'file:///local-profile.json',
        metadata:{camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:35},
        upload:false
    },'valid local export remains available without upload consent');
    advance(h.state,h.scheduled);
}

{
    const h=harness({upload:false,valid:true});
    h.state.batch.saveResult(5.1);
    assert.equal(h.state.picker,null,'poor calibration result skips export');
    advance(h.state,h.scheduled);
}

console.log('Calibration batch export regressions passed');
