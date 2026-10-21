// Focused checks for the production QML JavaScript, plus an opt-in lookup benchmark.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { performance } = require('node:perf_hooks');
const local = path.join(__dirname, 'CameraCatalog.js');
const source = fs.existsSync(local) ? local : path.join(__dirname, '../src/ui/components/CameraCatalog.js');
const context = vm.createContext({console});
vm.runInContext(fs.readFileSync(source, 'utf8').replace(/^\.pragma library\s*/, ''), context);
const plain = value => JSON.parse(JSON.stringify(value));
const document = {
  schema_version: 1,
  cameras: [
    {brand:'Sony', model:'ILCE-7M4', aliases:['Alpha 7 IV'], mounts:['Sony E'], crop_factor:1},
    {brand:'Sony', model:'ILCE-7M3', aliases:[], mounts:['Sony E'], crop_factor:1},
    {brand:'Sony', model:'ILCE-6000', aliases:['Alpha 6000'], mounts:['Sony E'], crop_factor:1.534},
    {brand:'Other', model:'Unknown sensor', aliases:[], mounts:['Sony E']}
  ],
  lenses: [
    {brand:'Sony', model:'FE 24–70mm F2.8', mounts:['Sony E'], crop_factor:1},
    {brand:'Sony', model:'E 16–50mm F3.5–5.6', mounts:['Sony E'], crop_factor:1.534}
  ],
  profiles: [
    {id:'profile-a', name:'Setup A', brand:'Sony', model:'ILCE-7M4', lens:'FE 24–70mm F2.8', checksum:'a', official:false},
    {id:'profile-b', name:'Setup B', brand:'Sony', model:'Alpha 7 IV', lens:'FE 24–70mm F2.8', checksum:'b', official:true}
  ]
};
const index = context.build(document);
assert.equal(context.prefill(index, {brand:'sony',model:'Alpha 7 IV'}).model, 'ILCE-7M4');
assert.deepEqual(
  plain(context.prefill(index, {model:'Alpha 7 IV'})),
  {brand:'Sony', model:'ILCE-7M4', lens:'', known:true}
);
assert.equal(context.prefill(index, {brand:'Canon', model:'ILCE-7M4'}).known, false);
const ambiguousUnbranded = context.build({
  schema_version:1,
  cameras:[
    {brand:'Sony', model:'Shared Body', aliases:[], mounts:[]},
    {brand:'Canon', model:'Shared Body', aliases:[], mounts:[]}
  ],
  lenses:[],
  profiles:[]
});
assert.equal(context.prefill(ambiguousUnbranded, {model:'Shared Body'}).known, false);
assert.equal(
  context.prefill(index, {
    brand:'Sony', model:'ILCE-7M4',
    lens_model:'Telemetry default', lens_info:'FE 24–70mm F2.8'
  }).lens,
  'FE 24–70mm F2.8'
);
assert.equal(
  context.prefill(index, {
    brand:'Sony', model:'ILCE-7M4',
    lens_model:'FE', lens_info:'24–70mm F2.8'
  }).lens,
  'FE 24–70mm F2.8'
);
assert.equal(
  context.prefill(index, {
    brand:'Sony', model:'ILCE-7M4',
    lens_model:'Manual prime', lens_info:'Unmatched metadata'
  }).lens,
  'Manual prime'
);
assert.equal(context.prefill(index, {brand:'Sony',model:'ILCE'}).known, false);
assert.deepEqual(plain(context.profilesFor(index,'Sony','ILCE-7M4','FE 24–70mm F2.8')).map(p=>p.id), ['profile-b','profile-a']);
console.log('PASS metadata alias + profile indexing');
// A user-selectable canonical ID must beat another model's alias regardless
// of source order, while two truly competing aliases stay ambiguous.
const collidingModels = [
  {brand:'Sony', model:'A', aliases:['Shared'], mounts:[]},
  {brand:'Sony', model:'B', aliases:['A','Shared'], mounts:[]}
];
for (const cameras of [collidingModels, collidingModels.slice().reverse()]) {
  const collisionIndex = context.build({schema_version:1, cameras, lenses:[], profiles:[]});
  assert.equal(context.resolveCamera(collisionIndex, 'Sony', 'A'), JSON.stringify(['sony','a']));
  assert.equal(context.resolveCamera(collisionIndex, 'Sony', 'B'), JSON.stringify(['sony','b']));
  assert.equal(context.resolveCamera(collisionIndex, 'Sony', 'Shared'), '');
}
console.log('PASS canonical camera IDs outrank colliding aliases');

// Remote metadata may be valid schema JSON but contain malformed individual
// entries. Ignore those without losing unrelated known cameras and profiles.
const malformedRecords = context.build({
  schema_version: 1,
  cameras: [null, false, [], {
    brand: 'Sony', model: 'Bad imported camera',
    aliases: 'not-an-array', mounts: 'not-an-array', crop_factor: 1
  }, ...document.cameras],
  lenses: [null, false, {brand:'Other', model:'Broken metadata', mounts:{value:'Sony E'}},
           ...document.lenses],
  profiles: [null, false, {id:42, brand:'Sony', model:'ILCE-7M4'},
             ...document.profiles]
});
assert.equal(context.prefill(malformedRecords, {brand:'Sony',model:'Alpha 7 IV'}).model, 'ILCE-7M4');
assert.deepEqual(plain(context.profilesFor(malformedRecords,'Sony','ILCE-7M4','FE 24–70mm F2.8')).map(p=>p.id),
                 ['profile-b','profile-a']);
assert.deepEqual(plain(context.lensLabels(malformedRecords,'Sony','Bad imported camera')), []);
assert.deepEqual(plain(context.compatibleCameras(malformedRecords,'Sony','Bad imported camera')), []);
console.log('PASS malformed downloaded records do not invalidate remaining catalog');



assert.equal(context.profilesFor(index,'Sony','ILCE-7M3','FE 24–70mm F2.8').length, 0);
assert.deepEqual(plain(context.compatibleCameras(index,'Sony','ILCE-7M4')).map(c=>c.model), ['ILCE-7M3']);
assert.equal(context.lensLabels(index,'Sony','ILCE-7M4').includes('E 16–50mm F3.5–5.6'), false);
console.log('PASS catalogue-only separation + crop/mount matching');
const profiles = context.profilesFor(index,'Sony','ILCE-7M4','FE 24–70mm F2.8');
assert.equal(context.visibleProfiles(profiles,{'profile-a':true},false).length, 1);
assert.equal(context.visibleProfiles(profiles,{'profile-a':true},true).length, 2);
assert.equal(context.profilesFor(index,'Sony','ILCE-7M4','FE 24–70mm F2.8').length, 2);
assert.throws(() => context.build({schema_version:2,cameras:[],lenses:[],profiles:[]}));
assert.equal(context.visibleProfiles([{id:'one',checksum:'same'},{id:'two',checksum:'same'}],{one:true},false).length,1);
const specialProfiles = [{id:'toString'}, {id:'constructor'}, {id:'__proto__'}];
assert.deepEqual(plain(context.visibleProfiles(specialProfiles, {}, false)).map(p=>p.id),
                 ['toString','constructor','__proto__']);
const specialHidden = context.normalizeHidden(JSON.parse('{"__proto__":true,"toString":true,"ignored":"yes"}'));
assert.equal(Object.getPrototypeOf(specialHidden), null);
assert.deepEqual(plain(context.visibleProfiles(specialProfiles, specialHidden, false)).map(p=>p.id),
                 ['constructor']);
delete specialHidden.__proto__;
assert.deepEqual(plain(context.visibleProfiles(specialProfiles, specialHidden, false)).map(p=>p.id),
                 ['constructor','__proto__']);
assert.deepEqual(plain(context.normalizeHidden([])), {});
console.log('PASS reversible review + invalid catalogue rejection');
if (process.argv.includes('--benchmark')) {
  const count = 50000, queries = 2000;
  const large = {schema_version:1,cameras:[],lenses:[],profiles:[]};
  for(let i=0;i<count;i++) {
    large.profiles.push({id:String(i),name:'Profile '+i,brand:'Benchmark',model:'Camera '+(i%1000),lens:'Lens '+(i%17),checksum:String(i)});
  }
  const buildStart=performance.now(), indexed=context.build(large), buildMs=performance.now()-buildStart;
  let found=0;
  const fastStart=performance.now();
  for(let i=0;i<queries;i++) found+=context.profilesFor(indexed,'Benchmark','Camera '+(i%1000),'Lens '+(i%17)).length;
  const indexedMs=performance.now()-fastStart;
  let linearFound=0;
  const linearStart=performance.now();
  for(let i=0;i<queries;i++) linearFound+=large.profiles.filter(p=>p.brand==='Benchmark'&&p.model==='Camera '+(i%1000)&&p.lens==='Lens '+(i%17)).length;
  const linearMs=performance.now()-linearStart;
  assert.equal(found,linearFound);
  console.log(JSON.stringify({benchmark:'synthetic exact-selection lookup, not full-application rendering',records:count,queries,build_ms:buildMs,indexed_ms:indexedMs,linear_ms:linearMs,matches:found}));
}
