// Execute QML review logic with explicit doubles for the native loading boundary.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source = readFileSync(new URL('../src/ui/menu/LensProfile.qml', import.meta.url), 'utf8');
const videoSource = readFileSync(new URL('../src/ui/VideoArea.qml', import.meta.url), 'utf8');
function methodFrom(code, name, indent = '    ') {
    const start = code.indexOf(`${indent}function ${name}(`);
    assert.ok(start >= 0, `Missing ${name}`);
    const end = code.indexOf(`\n${indent}}`, start) + indent.length + 2;
    return code.slice(start, end).replace(/: (string|bool|var|void|int|list<var>)(?=[,)\s{])/g, '');
}
function method(name, indent = '    ') {
    return methodFrom(source, name, indent);
}
function harness(contents = '{}', accepted = true, loadedFilename = 'clip.mp4', loading = false) {
    const state = {
        root: {
            reviewProfiles:[],reviewIndex:-1,presetLoadBusy:false,activeProfileLoad:null,queuedProfileLoad:null,
            pendingGyroflowData:null,pendingQueueJobId:0
        },
        profileLoadExpiry: {restart() {}, stop() {}},
        controller: {
            get_preset_contents:()=>contents,
            load_lens_profile:()=>accepted,
            loading_gyro_in_progress:loading,
            cancel_current_operation() {},
            get_urls_from_gyroflow_file:()=>[],
            get_version_from_gyroflow_file:()=>1,
            image_sequence_fps:0,
            set_prevent_recompute() {},
            import_gyroflow_file() {},
            imported:false,
            import_gyroflow_data() { this.imported=true; },
            telemetryArgs:null,
            load_telemetry(...args) { this.telemetryArgs=args; },
        },
        filesystem: {get_filename:url=>String(url).split('/').pop(),exists:()=>true},
        vidInfo: {filename:loadedFilename},
        render_queue: {editing_job_id:0},
        vid: {setFrameRate() {}},
        window: {
            motionData:{filename:'',allMetadata:false,lastSelectedFile:'',setAllMetadata(value) { this.allMetadata=value; }},
            videoArea:{}
        },
        qsTr:s=>s, Modal:{Error:1}, messageBox() {}, console:{warn() {},log() {}},
    };
    const context=vm.createContext(state);
    vm.runInContext(methodFrom(videoSource, 'loadGyroflowData')+'\n'+method('loadProfileItem')+'\n'+method('reviewRelative'),context);
    state.window.videoArea.loadGyroflowData = (obj, id) => context.loadGyroflowData(obj, id);
    return context;
}
let context=harness('{broken');
context.loadProfileItem(['Bad preset','bad.gyroflow','crc']);
assert.equal(context.root.presetLoadBusy,false,'malformed preset must not wedge later selection');
assert.equal(context.root.activeProfileLoad,null,'malformed preset must not contaminate the next profile identity');
context=harness('{}',false);
context.loadProfileItem(['Missing profile','missing.json','crc']);
assert.equal(context.root.activeProfileLoad,null,'failed native load must clear pending identity');
context=harness('{}',true,'');
context.loadProfileItem(['Rejected preset','rejected.gyroflow','rejected']);
assert.equal(context.root.presetLoadBusy,false,'a preset rejected before loading a video must release review state');
assert.equal(context.root.activeProfileLoad,null,'a rejected preset must clear its pending profile identity');
context=harness('{"videofile":"clip.mp4"}');
context.loadProfileItem(['Accepted preset','accepted.gyroflow','accepted']);
assert.equal(context.root.presetLoadBusy,true,'an accepted preset must retain review state until the load callback');
assert.equal(context.controller.imported,true,'accepted preset data reaches the Gyroflow importer');
context=harness('{}',true,'clip.mp4');
assert.equal(context.loadGyroflowData({videofile:'clip.mp4',gyro_source:{filepath:'gyro.mp4',all_metadata:true}},7),true,'a preset with separate gyro data is accepted');
assert.equal(context.window.motionData.allMetadata,true,'preset metadata policy is applied before gyro loading');
assert.equal(context.controller.telemetryArgs[1],true,'the gyro loader receives the requested full metadata');
context=harness('{}',true,'clip.mp4',true);
assert.equal(context.loadGyroflowData({},7),true,'a preset queued behind active telemetry counts as accepted');
assert.equal(context.root.pendingQueueJobId,7,'queued preset retains its queue job identity');
context=harness();
context.root.presetLoadBusy=true;
const first=['First','first.json','one'], latest=['Latest','latest.json','two'];
context.loadProfileItem(first);
context.loadProfileItem(latest);
assert.equal(context.root.queuedProfileLoad,latest,'rapid choices retain only the newest pending selection');
context=harness();
vm.runInContext(method('onSearch_lens_profile_finished','        '),context);
context.root.profilePath='chosen.json'; context.root.profileChecksum='chosen';
context.onSearch_lens_profile_finished([['First','first.json','first'],['Chosen','chosen.json','chosen']]);
assert.equal(context.root.reviewIndex,1,'refresh preserves the selected review position');
context.onSearch_lens_profile_finished([['Other','other.json','other']]);
assert.equal(context.root.reviewIndex,-1,'a filtered-out selection does not impersonate a result');
console.log('Profile review regressions passed');
