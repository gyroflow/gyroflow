
function getFolder(v) {
    let idx = v.lastIndexOf("/");
    if (idx == -1) idx = v.lastIndexOf("\\");
    if (idx == -1) return "";
    return v.substring(0, idx + 1);
}
function getFilename(v) {
    let idx = v.lastIndexOf("/");
    if (idx == -1) idx = v.lastIndexOf("\\");
    return v.substring(idx + 1);
}
function timeToStr(v) {
    const d = Math.floor((v %= 31536000) / 86400),
          h = Math.floor((v %= 86400) / 3600),
          m = Math.floor((v %= 3600) / 60),
          s = Math.round(v % 60);
    if (d || h || m || s) {
        return (d? d + qsTr("d") + " " : "") +
               (h? h + qsTr("h") + " " : "") +
               (m? m + qsTr("m") + " " : "") +
                   s + qsTr("s");
    }
    return qsTr("&lt; 1s");
}
function calculateTimesAndFps(progress, current_frame, start_timestamp, end_timestamp) {
    if (typeof end_timestamp === "undefined") end_timestamp = Date.now();
    if (progress > 0 && progress <= 1.0 && start_timestamp > 0) {
        const elapsedMs = end_timestamp - start_timestamp;
        const totalEstimatedMs = elapsedMs / progress;
        const remainingMs = totalEstimatedMs - elapsedMs;
        let ret = [];
        if (remainingMs > 5 || elapsedMs > 5) {
            ret[0] = timeToStr(elapsedMs / 1000);
            ret[1] = timeToStr(remainingMs / 1000);
        }
        if (elapsedMs > 5 && current_frame > 0) {
            ret[2] = current_frame / (elapsedMs / 1000.0);
        }
        return ret.length? ret : false;
    } else {
        return false;
    }
}
