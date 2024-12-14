function setupAudioLevelMonitoring(stream) {
    const audioContext = new (window.AudioContext || window.webkitAudioContext)();
    const source = audioContext.createMediaStreamSource(stream);
    const analyser = audioContext.createAnalyser();
    
    analyser.fftSize = 2048;
    analyser.minDecibels = -90;
    analyser.maxDecibels = -10;
    analyser.smoothingTimeConstant = 0.85;
    
    source.connect(analyser);
    
    const dataArray = new Uint8Array(analyser.frequencyBinCount);
    
    function checkAudioLevel() {
        analyser.getByteFrequencyData(dataArray);
        const average = dataArray.reduce((a, b) => a + b) / dataArray.length;
        
        if (average > 0) {
            console.log('Audio level detected:', {
                average,
                stream: stream.id,
                tracks: stream.getTracks().map(t => ({
                    id: t.id,
                    kind: t.kind,
                    enabled: t.enabled,
                    muted: t.muted
                }))
            });
        }
        requestAnimationFrame(checkAudioLevel);
    }
    
    checkAudioLevel();
    
    if (audioContext.state === 'suspended') {
        audioContext.resume();
    }
}

function setupVolumeIndicator() {
    const volumeDiv = document.createElement('div');
    volumeDiv.style.padding = '10px';
    volumeDiv.style.margin = '10px';
    volumeDiv.style.border = '1px solid #ccc';
    document.body.appendChild(volumeDiv);
    
    return volumeDiv;
}

function monitorAudioState(stream) {
    const audioContext = new (window.AudioContext || window.webkitAudioContext)();
    const source = audioContext.createMediaStreamSource(stream);
    const analyser = audioContext.createAnalyser();
    const meter = createAudioMeter();
    
    analyser.fftSize = 1024;
    analyser.minDecibels = -90;
    analyser.maxDecibels = -10;
    analyser.smoothingTimeConstant = 0.5;
    
    source.connect(analyser);
    
    const dataArray = new Uint8Array(analyser.frequencyBinCount);
    let isLocal = stream === localStream;
    console.log(`Monitoring ${isLocal ? 'local' : 'remote'} audio stream:`, stream.id);
    
    function checkAudioLevel() {
        analyser.getByteFrequencyData(dataArray);
        const average = dataArray.reduce((a, b) => a + b) / dataArray.length;
        const normalizedLevel = average / 255;
        
        if (isLocal) {
            meter.updateLocalLevel(normalizedLevel);
        } else {
            meter.updateRemoteLevel(normalizedLevel);
        }

        if (normalizedLevel > 0.01) {
            console.log(`Audio level (${isLocal ? 'local' : 'remote'}):`, {
                level: normalizedLevel.toFixed(3),
                streamId: stream.id,
                tracks: stream.getTracks().map(t => ({
                    id: t.id,
                    kind: t.kind,
                    enabled: t.enabled,
                    muted: t.muted,
                    readyState: t.readyState
                }))
            });
        }
        requestAnimationFrame(checkAudioLevel);
    }
    
    checkAudioLevel();
    
    if (audioContext.state === 'suspended') {
        audioContext.resume().then(() => {
            console.log('AudioContext resumed');
        });
    }

    return () => {
        audioContext.close();
    };
}

function createAudioMeter() {
    const meterContainer = document.createElement('div');
    meterContainer.style.cssText = `
        position: fixed;
        bottom: 20px;
        right: 20px;
        background: #333;
        padding: 10px;
        border-radius: 5px;
        color: white;
    `;

    const localMeter = document.createElement('div');
    localMeter.innerHTML = `
        <div>Local Audio Level:</div>
        <div class="meter" style="
            width: 200px;
            height: 20px;
            background: #555;
            border-radius: 10px;
            overflow: hidden;
        ">
            <div class="level" style="
                width: 0%;
                height: 100%;
                background: #4CAF50;
                transition: width 0.1s;
            "></div>
        </div>
    `;

    const remoteMeter = document.createElement('div');
    remoteMeter.innerHTML = `
        <div>Remote Audio Level:</div>
        <div class="meter" style="
            width: 200px;
            height: 20px;
            background: #555;
            border-radius: 10px;
            overflow: hidden;
            margin-top: 10px;
        ">
            <div class="level" style="
                width: 0%;
                height: 100%;
                background: #2196F3;
                transition: width 0.1s;
            "></div>
        </div>
    `;

    meterContainer.appendChild(localMeter);
    meterContainer.appendChild(remoteMeter);
    document.body.appendChild(meterContainer);

    return {
        updateLocalLevel: (level) => {
            const percentage = Math.min(100, level * 400);
            localMeter.querySelector('.level').style.width = `${percentage}%`;
        },
        updateRemoteLevel: (level) => {
            const percentage = Math.min(100, level * 400);
            remoteMeter.querySelector('.level').style.width = `${percentage}%`;
        }
    };
} 

async function checkMediaDevices() {
    try {
        const mediaControls = document.getElementById('mediaControls');
        if (navigator.mediaDevices && navigator.mediaDevices.getUserMedia) {
            mediaControls.style.display = 'block';
            console.log('Media devices available');
        } else {
            console.error('Media devices not available');
            mediaControls.style.display = 'none';
        }
    } catch (err) {
        console.error('Error checking media devices:', err);
    }
} 