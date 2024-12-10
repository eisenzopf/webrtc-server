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
        audioContext.resume();
    }
} 