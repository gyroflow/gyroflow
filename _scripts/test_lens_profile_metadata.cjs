#!/usr/bin/env node
'use strict';

// Exercise the actual LensProfile.qml JavaScript snippets without a Qt desktop
// build or a vendored duplicate of the production formatting implementation.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const qml = fs.readFileSync(path.join(__dirname, '../src/ui/menu/LensProfile.qml'), 'utf8');
function section(start, end) {
  const from = qml.indexOf(start);
  const to = qml.indexOf(end, from + start.length);
  assert.ok(from >= 0 && to > from, 'expected LensProfile source section');
  return qml.slice(from, to);
}

const numeric = section('const focalLength = Number(obj.focal_length);', 'if (obj.asymmetrical)');
const crop = section('root.cropFactor = Number.isFinite(cropFactor)', 'if (!root.selected_manually');
const format = new Function('obj', 'lensInfo', 'root', [
  numeric, crop,
  'return { info: lensInfo, factor: root.cropFactor };',
].join('\n'));

let result = format(
  { focal_length: '35.4', crop_factor: '1.5' },
  {}, { cropFactor: 0 }
);
assert.equal(result.info['Focal length'], '35.40 mm');
assert.equal(result.info['Crop factor'], '1.50x');
assert.equal(result.factor, 1.5);

result = format({ focal_length: 24, crop_factor: 1.2 }, {}, { cropFactor: 0 });
assert.equal(result.info['Focal length'], '24.00 mm');
assert.equal(result.factor, 1.2);

result = format({ focal_length: 'Infinity', crop_factor: 'NaN' }, {}, { cropFactor: 1 });
assert.equal(result.info['Focal length'], undefined);
assert.equal(result.info['Crop factor'], undefined);
assert.equal(result.factor, 0);

const telemetry = section(
  'const identifier = additional_data && additional_data.camera_identifier;',
  'cameraSelector.setMetadata(identifier || {});'
) + 'cameraSelector.setMetadata(identifier || {});';

let calls = [];
const selector = { setMetadata(value) { calls.push(value); } };
const update = new Function('additional_data', 'cameraSelector', telemetry);
update(null, selector);
update(undefined, selector);
update({}, selector);
update({ camera_identifier: { model: 'EOS' } }, selector);
assert.equal(calls.length, 4);
assert.deepEqual(calls[0], {});
assert.deepEqual(calls[1], {});
assert.deepEqual(calls[2], {});
assert.deepEqual(calls[3], { model: 'EOS' });

console.log('Focused LensProfile metadata compatibility checks passed');
