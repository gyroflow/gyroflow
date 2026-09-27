// Execute QML review logic with explicit doubles for the native loading boundary.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source = readFileSync(new URL('../src/ui/menu/LensProfile.qml', import.meta.url), 'utf8');
function method(name, indent = '    ') {
    const start = source.indexOf(`${indent}function ${name}(`);
    assert.ok(start >= 0, `Missing ${name}`);
    const end = source.indexOf(`\n${indent}}`, start) + indent.length + 2;
    return source.slice(start, end).replace(/: (string|bool|var|void|int|list<var>)(?=[,)\s{])/g, '');
}
function harness(contents = '{}', accepted = true) {
    const state = {
        root: {reviewProfiles:[],reviewIndex:-1,presetLoadBusy:false,activeProfileLoad:null,queuedProfileLoad:null},
        profileLoadExpiry: {restart() {}, stop() {}},
        controller: {get_preset_contents:()=>contents,load_lens_profile:()=>accepted},
        window: {videoArea:{loadGyroflowData:()=>accepted}},
        qsTr:s=>s, Modal:{Error:1}, messageBox() {}, console:{warn() {}},
    };
    const context=vm.createContext(state);
    vm.runInContext(method('loadProfileItem')+'\n'+method('reviewRelative'),context);
    return context;
}
let context=harness('{broken');
context.loadProfileItem(['Bad preset','bad.gyroflow','crc']);
assert.equal(context.root.presetLoadBusy,false,'malformed preset must not wedge later selection');
assert.equal(context.root.activeProfileLoad,null,'malformed preset must not contaminate the next profile identity');
context=harness('{}',false);
context.loadProfileItem(['Missing profile','missing.json','crc']);
assert.equal(context.root.activeProfileLoad,null,'failed native load must clear pending identity');
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
