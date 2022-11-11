// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

import QtQuick

import Gyroflow

Item {
    property VideoArea videoArea;

    // Play/Pause
    Shortcut {
        sequence: "Space";
        onActivated: {
            videoArea.timeline.focus = true;
            if (videoArea.vid.playing) videoArea.vid.pause();
            else                       videoArea.vid.play();
        }
    }
    // Previous frame
    Shortcut {
        sequences: ["Left", "Page Up", ","];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame -= 1;
        }
    }
    // Previous 10 frames
    Shortcut {
        sequences: ["Ctrl+Left", "Ctrl+Page Up", "Ctrl+,"];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame -= 10;
        }
    }
    // Next frame
    Shortcut {
        sequences: ["Right", "Page Down","."];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame += 1;
        }
    }
    // Next 10 frames
    Shortcut {
        sequences: ["Ctrl+Right", "Ctrl+Page Down","Ctrl+."];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame += 10;
        }
    }
    // Go to trim start
    Shortcut {
        sequence: "Home";
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.trimStart);
        }
    }
    // Go to trim end
    Shortcut {
        sequence: "End";
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.trimEnd);
        }
    }
    // Set trim start here
    Shortcut {
        sequences: ["i", "["];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.timeline.setTrim(videoArea.timeline.position, videoArea.timeline.trimEnd);
        }
    }
    // Set trim end here
    Shortcut {
        sequences: ["o", "]"];
        onActivated: {
            videoArea.timeline.focus = true;
            videoArea.timeline.setTrim(videoArea.timeline.trimStart, videoArea.timeline.position);
        }
    }
    // Mute on/off
    Shortcut {
        sequence: "m";
        onActivated: videoArea.vid.muted = !videoArea.vid.muted;
    }
    // Stabilization on/off
    Shortcut {
        sequence: "s";
        onActivated: videoArea.stabEnabledBtn.checked = !videoArea.stabEnabledBtn.checked;
    }

    // Hide chart axis X
    Shortcut {
        sequence: "x";
        onActivated: videoArea.timeline.toggleAxis(0, false);
    }
    // Hide chart axis Y
    Shortcut {
        sequence: "y";
        onActivated: videoArea.timeline.toggleAxis(1, false);
    }
    // Hide chart axis Z
    Shortcut {
        sequence: "z";
        onActivated: videoArea.timeline.toggleAxis(2, false);
    }
    // Hide chart axis W
    Shortcut {
        sequence: "w";
        onActivated: videoArea.timeline.toggleAxis(3, false);
    }

    // Show chart axis X
    Shortcut {
        sequence: "Shift+x";
        onActivated: videoArea.timeline.toggleAxis(0, true);
    }
    // Show chart axis Y
    Shortcut {
        sequence: "Shift+y";
        onActivated: videoArea.timeline.toggleAxis(1, true);
    }
    // Show chart axis Z
    Shortcut {
        sequence: "Shift+z";
        onActivated: videoArea.timeline.toggleAxis(2, true);
    }
    // Show chart axis W
    Shortcut {
        sequence: "Shift+w";
        onActivated: videoArea.timeline.toggleAxis(3, true);
    }

    // Chart display mode: Gyroscope
    Shortcut {
        sequence: "shift+g";
        onActivated: videoArea.timeline.setDisplayMode(0);
    }
    // Chart display mode: Accelerometer
    Shortcut {
        sequence: "shift+a";
        onActivated: videoArea.timeline.setDisplayMode(1);
    }
    // Chart display mode: Magnetometer
    Shortcut {
        sequence: "shift+m";
        onActivated: videoArea.timeline.setDisplayMode(2);
    }
    // Chart display mode: Quaternions
    Shortcut {
        sequence: "shift+q";
        onActivated: videoArea.timeline.setDisplayMode(3);
    }

    // Next keyframe
    Shortcut {
        sequence: "Shift+Right";
        onActivated: videoArea.timeline.jumpToNextKeyframe("");
    }
    // Previous keyframe
    Shortcut {
        sequence: "Shift+Left";
        onActivated: videoArea.timeline.jumpToPrevKeyframe("");
    }

    // Timeline: Auto sync here
    Shortcut {
        sequence: "a";
        onActivated: videoArea.timeline.addAutoSyncPoint(videoArea.timeline.position);
    }
    // Timeline: Add manual sync point here
    Shortcut {
        sequence: "p";
        onActivated: videoArea.timeline.addManualSyncPoint(videoArea.timeline.position);
    }

    // Exit full screen mode
    Shortcut {
        sequence: "Esc";
        onActivated: videoArea.fullScreen = false;
    }

    // Toggle render queue
    Shortcut {
        sequence: "q";
        onActivated: videoArea.queue.shown = !videoArea.queue.shown;
    }

    // Save project file
    Shortcut {
        sequence: "Ctrl+s";
        onActivated: window.saveProject();
    }
}
