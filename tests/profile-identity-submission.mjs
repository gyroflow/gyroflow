// Exercise the QML-shared pure camera identity, review and submission modules.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';

function moduleAt(path) {
    const context = vm.createContext({});
    vm.runInContext(readFileSync(new URL(path, import.meta.url), 'utf8'), context);
    return context;
}

const camera = moduleAt('../src/ui/menu/CameraIdentity.js');
const review = moduleAt('../src/ui/menu/ProfileReview.js');
const submission = moduleAt('../src/ui/menu/ProfileSubmission.js');

assert.deepEqual(Array.from(camera.selectorModel('Any', ['Sony', '', 'Sony', 'Canon'], 'Other')), ['Any', 'Sony', 'Canon', 'Other']);
assert.deepEqual(Array.from(camera.selectorModel('Other', ['Sony'], '')), ['Other', 'Sony']);
assert.equal(camera.selectorValue(0, ['Other', 'Sony'], false), '');
assert.equal(camera.selectorValue(1, ['Other', 'Sony'], false), 'Sony');
assert.equal(camera.selectorValue(2, ['Any', 'Sony', 'Other'], true), '');
assert.equal(camera.selectorIndex(['Other', 'Sony'], 'Sony'), 1);
assert.deepEqual(JSON.parse(JSON.stringify(camera.detectedCamera({brand:'Sony', model:'A7', lens_model:'FE', lens_info:'Zoom'}))),
    {brand:'Sony', model:'A7', lens:'FE'});
assert.deepEqual(JSON.parse(JSON.stringify(camera.detectedCamera({}))), {});

assert.equal(camera.cameraNeedsLens({fixed_lens:true}), false, 'catalog-marked fixed lens is exempt');
assert.equal(camera.cameraNeedsLens({}), true, 'unknown catalog data conservatively requires a lens');
assert.equal(camera.cameraNeedsLens({mounts:['lowercaseFixedMount']}), true, 'mount naming does not substitute for catalog identity');
assert.equal(submission.hasSetupName('Sony'), true);
for (const value of ['', 'Other', 'unknown', 'n/a', '---']) assert.equal(submission.hasSetupName(value), false);
for (const lens of ['24-70mm', '24–70mm', 'Power Zoom', '18-105 F4', '16-35']) assert.equal(submission.isZoomLens(lens), true, lens);
assert.equal(submission.isZoomLens('35mm F1.8'), false);

const profiles = [['First','one.json','one'], ['Chosen','chosen.json','chosen']];
assert.equal(review.findIndex(profiles, 'chosen.json', ''), 1, 'path preserves selection');
assert.equal(review.findIndex(profiles, '', 'chosen'), 1, 'checksum preserves selection');
assert.equal(review.findIndex(profiles, 'gone.json', 'gone'), -1, 'missing identity does not impersonate a result');
assert.equal(review.findExactIndex(profiles, 'chosen.json', 'chosen'), 1);
assert.deepEqual(JSON.parse(JSON.stringify(review.withoutIdentity(profiles, 'one', ''))), [profiles[1]]);

function validate(metadata, overrides = {}) {
    const selection = Object.assign({brandIndex:1,brandValue:'Sony',modelIndex:0,modelValue:'',lensIndex:0,lensValue:'',hasFocalLength:true}, overrides);
    return Array.from(submission.validationErrors(metadata, selection, true));
}
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:35}), ['lens_name_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'35mm f/2',focal_length:35}), ['lens_name_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'f/2.8'}), ['lens_name_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'Sony FE 24-70mm f/2.8',focal_length:35}), []);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'Sigma 24-70mm f/2.8',focal_length:35}), []);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:35},{lensIndex:1,lensValue:'24-70mm'}), []);
assert.deepEqual(Array.from(submission.validationErrors({camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:35},
    {brandIndex:1,brandValue:'Sony',modelIndex:0,modelValue:'',lensIndex:0,lensValue:'',hasFocalLength:true}, false)), [],
    'fixed-lens bodies do not require manual lens identification');
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'24-70mm',focal_length:0},{hasFocalLength:false}), ['lens_name_required','zoom_focal_length_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:''}), ['lens_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'Other',lens_model:''},{brandIndex:0}), ['camera_model_required','lens_required']);
assert.deepEqual(validate({camera_brand:'Canon',camera_model:'A7',lens_model:'35mm'},{brandIndex:2,brandValue:''}), ['camera_other_required','lens_name_required']);
assert.deepEqual(validate({camera_brand:'Sony',camera_model:'A7',lens_model:'Custom'},{lensIndex:1,lensValue:'Catalog lens'}), ['lens_other_required']);
const metadata = {camera_brand:'Sony', nested:{value:1}};
const snapshot = submission.metadataSnapshot(metadata);
snapshot.nested.value = 2;
assert.equal(metadata.nested.value, 1, 'export metadata is an independent snapshot');

console.log('Profile identity and submission module regressions passed');
