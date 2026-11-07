// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

function matches(item, path, checksum) {
    return !!item && (item[1] === path || (!!checksum && item[2] === checksum));
}

function findIndex(profiles, path, checksum) {
    return (profiles || []).findIndex(item => matches(item, path, checksum));
}

function findExactIndex(profiles, path, checksum) {
    return (profiles || []).findIndex(item => item[1] === path && item[2] === checksum);
}

function withoutIdentity(profiles, checksum, path) {
    return (profiles || []).filter(item => !(checksum && item[2] === checksum) && !(path && item[1] === path));
}
