import assert from 'node:assert/strict';

// Run QML handlers with boundary doubles, stripping type annotations only from the signature.
export function qmlMethod(source, name, indent = '    ') {
    const signature = indent + 'function ' + name + '(';
    const start = source.indexOf(signature);
    assert.notEqual(start, -1, 'Missing QML function: ' + name);
    const body = source.indexOf('{', start + signature.length);
    const end = source.indexOf('\n' + indent + '}', body);
    assert.ok(body > start && end > body, 'Incomplete QML function: ' + name);
    const js = source.slice(start, body).replace(/: (?:string|bool|var|void|int|real|url|list<var>)(?=[,)\s])/g, '');
    return js + source.slice(body, end + indent.length + 2);
}
