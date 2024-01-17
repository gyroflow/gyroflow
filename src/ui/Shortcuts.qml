// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

import QtQuick

import Gyroflow

Item {
    property VideoArea videoArea;

    // Play/Pause
    Shortcut {
        sequences: ["Space", "F3"];
        onActivated: {
            if (videoArea.vid.playing) videoArea.vid.pause();
            else                       videoArea.vid.play();
        }
    }
    // Previous frame
    Shortcut {
        sequences: ["Left", "Page Up", ",", "F2"];
        onActivated:  videoArea.vid.seekToFrameDelta(-1);
    }
    // Previous 10 frames
    Shortcut {
        sequences: ["Ctrl+Left", "Ctrl+Page Up", "Ctrl+,", "F5"];
        onActivated: videoArea.vid.seekToFrameDelta(-10);
    }
    // Next frame
    Shortcut {
        sequences: ["Right", "Page Down", ".", "F4"];
        onActivated: videoArea.vid.seekToFrameDelta(1);
    }
    // Next 10 frames
    Shortcut {
        sequences: ["Ctrl+Right", "Ctrl+Page Down", "Ctrl+.", "F6"];
        onActivated: videoArea.vid.seekToFrameDelta(10);
    }
    // Go to trim start
    Shortcut {
        sequence: "Home";
        onActivated: {
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.getTrimRanges()[0][0]) + 1;
        }
    }
    // Go to trim end
    Shortcut {
        sequence: "End";
        onActivated: {
            videoArea.vid.currentFrame = videoArea.timeline.frameAtPosition(videoArea.timeline.getTrimRanges()[0][1] || 1) - 1;
        }
    }
    // Set trim start here
    Shortcut {
        sequences: ["i", "["];
        onActivated: {
            videoArea.timeline.setTrimStart(videoArea.timeline.closestTrimRange(videoArea.timeline.position, true), videoArea.timeline.position);
        }
    }
    // Set trim end here
    Shortcut {
        sequences: ["o", "]"];
        onActivated: {
            videoArea.timeline.setTrimEnd(videoArea.timeline.closestTrimRange(videoArea.timeline.position, false), videoArea.timeline.position);
        }
    }
    // Add new trim start here
    Shortcut {
        sequences: ["Ctrl+i", "Ctrl+["];
        onActivated: {
            videoArea.timeline.addTrimStart(videoArea.timeline.position);
        }
    }
    // Add new trim end here
    Shortcut {
        sequences: ["Ctrl+o", "Ctrl+]"];
        onActivated: {
            videoArea.timeline.addTrimEnd(videoArea.timeline.position);
        }
    }
    // Clear trim range
    Shortcut {
        sequence: "c";
        onActivated: videoArea.timeline.resetTrim();
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

    // Stabilization overview on/off
    Shortcut {
        sequence: "d";
        onActivated: videoArea.fovOverviewBtn.checked = !videoArea.fovOverviewBtn.checked;
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
        onActivated: videoArea.fullScreen = 0;
    }

    // Toggle full screen mode
    Shortcut {
        sequence: "F11";
        onActivated: videoArea.fullScreen = (videoArea.fullScreen + 1) % 3;
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

    // Toggle grid guide
    Shortcut {
        sequence: "G";
        onActivated: window.videoArea.gridGuide.shown = !window.videoArea.gridGuide.shown;
    }
    // Grid guide color white/black
    Shortcut {
        sequence: "Ctrl+G";
        onActivated: window.videoArea.gridGuide.isBlack = !window.videoArea.gridGuide.isBlack;
    }

    // Play backwards
    Shortcut {
        id: j;
        sequence: "J";
        property int currentX: 1;
        onActivated: {
            //videoArea.vid.playbackRate = -1 * [1, 2, 4, 8, 16][currentX++ % 5];
            videoArea.vid.seekToFrameDelta(-100);
            videoArea.vid.play();
        }
    }
    // Play/Pause + reset playback rate
    Shortcut {
        sequences: ["K"];
        onActivated: {
            videoArea.vid.playbackRate = 1;
            j.currentX = l.currentX = 0;
            if (videoArea.vid.playing) videoArea.vid.pause();
            else                       videoArea.vid.play();
        }
    }
    // Play forward
    Shortcut {
        id: l;
        sequence: "L";
        property int currentX: 1;
        onActivated: { videoArea.vid.playbackRate = 1 * [1, 2, 4, 8, 16][currentX++ % 5]; videoArea.vid.play(); }
    }

    // Horizon lock roll adjustment shortcuts
    function hlRollAdjust(v: real) {
        if (window.stab.horizonCb.checked) {
            window.stab.horizonRollSlider.field.value += v;
        }
    }
    Shortcut { sequence: "E";       onActivated: hlRollAdjust(0.5);  }
    Shortcut { sequence: "Ctrl+E";  onActivated: hlRollAdjust(0.1);  }
    Shortcut { sequence: "Alt+E";   onActivated: hlRollAdjust(1);    }
    Shortcut { sequence: "Shift+E"; onActivated: hlRollAdjust(5);    }
    Shortcut { sequence: "R";       onActivated: hlRollAdjust(-0.5); }
    Shortcut { sequence: "Ctrl+R";  onActivated: hlRollAdjust(-0.1); }
    Shortcut { sequence: "Alt+R";   onActivated: hlRollAdjust(-1);   }
    Shortcut { sequence: "Shift+R"; onActivated: hlRollAdjust(-5);   }
}
