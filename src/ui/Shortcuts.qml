import QtQuick

import Gyroflow

Item {
    property VideoArea videoArea;

    Shortcut {
        sequence: "Space";
        onActivated: {
            videoArea.timeline.focus = true;
            if (videoArea.vid.playing)
                videoArea.vid.pause();
            else
                videoArea.vid.play();
        }
    }    
    Shortcut {
        sequences: ["Left", "Page Up", ","];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame -= 1;
        }
    }
    Shortcut {
        sequences: ["Ctrl+Left", "Ctrl+Page Up", "Ctrl+,"];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame -= 10;
        }
    }
    Shortcut {
        sequences: ["Right", "Page Down","."];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame += 1;
        }
    }
    Shortcut {
        sequences: ["Ctrl+Right", "Ctrl+Page Down","Ctrl+."];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame += 10;
        }
    }
    Shortcut {
        sequence: "Home";
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.trimStart);
        }
    }
    Shortcut {
        sequence: "End";
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.trimEnd);
        }
    }
    Shortcut {
        sequences: ["i", "["];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.timeline.setTrim(videoArea.timeline.position, videoArea.timeline.trimEnd);
        }
    }
    Shortcut {
        sequences: ["o", "]"];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.timeline.setTrim(videoArea.timeline.trimStart, videoArea.timeline.position);
        }
    }
    Shortcut {
        sequence: "m";
        onActivated: videoArea.vid.muted = !videoArea.vid.muted;
    }
    Shortcut {
        sequence: "s";
        onActivated: videoArea.stabEnabledBtn.checked = !videoArea.stabEnabledBtn.checked;
    }
}