export function updateStatus(message, isError = false, peerConnection = null) {
    const statusElement = document.getElementById('connectionStatus');
    if (statusElement) {
        statusElement.textContent = message;
        statusElement.className = isError ? 'status error' : 'status';
    }
    
    // Add connection state information if available
    if (peerConnection) {
        message += ` (ICE: ${peerConnection.iceConnectionState}, Connection: ${peerConnection.connectionState})`;
    }
    
    console.log('Status update:', {
        message, 
        isError, 
        peerConnection: peerConnection ? {
            iceConnectionState: peerConnection.iceConnectionState,
            connectionState: peerConnection.connectionState,
            signalingState: peerConnection.signalingState,
            iceGatheringState: peerConnection.iceGatheringState
        } : null
    });
}

function updateCallStatus(state, peer = null) {
    const statusMsg = peer ? `${state} with ${peer}` : state;
    updateStatus(statusMsg);
    
    if (state === 'connected') {
        document.getElementById('audioStatus').textContent = 'Audio status: Connected';
        document.getElementById('connectionStatus').className = 'status success';
    } else if (state === 'disconnected' || state === 'failed') {
        document.getElementById('audioStatus').textContent = 'Audio status: Not connected';
        document.getElementById('connectionStatus').className = 'status error';
    }
}

export function handlePeerListMessage(message) {
    console.log("Handling peer list update:", message);
    const currentPeerId = document.getElementById('peerId').value;
    const otherPeers = message.peers.filter(p => p !== currentPeerId);
    console.log("Filtered peers (excluding self):", otherPeers);
    
    const peerListDiv = document.getElementById('selectablePeerList');
    peerListDiv.innerHTML = otherPeers.map(peerId => `
        <div class="peer-item">
            <input type="checkbox" id="peer_${peerId}" value="${peerId}">
            <label for="peer_${peerId}">${peerId}</label>
        </div>
    `).join('');
    
    console.log("Updated peer list HTML");
}

function toggleMute() {
    if (localStream) {
        localStream.getAudioTracks().forEach(track => {
            track.enabled = isMuted;
            console.log(`Audio track ${track.id} enabled:`, track.enabled);
        });
        isMuted = !isMuted;
        document.getElementById('muteButton').textContent = isMuted ? 'Unmute' : 'Mute';
    }
}

function showCallAlert(caller) {
    return new Promise((resolve) => {
        const result = window.confirm(`Incoming call from ${caller}. Accept?`);
        resolve(result);
    });
}

function addDebugButton() {
    const button = document.createElement('button');
    button.textContent = 'Debug Connection';
    button.onclick = async () => {
        if (!peerConnection) {
            console.log('No peer connection exists');
            return;
        }

        console.log('Connection States:', {
            iceConnectionState: peerConnection.iceConnectionState,
            connectionState: peerConnection.connectionState,
            signalingState: peerConnection.signalingState,
            iceGatheringState: peerConnection.iceGatheringState
        });

        const stats = await peerConnection.getStats();
        stats.forEach(report => {
            if (report.type === 'candidate-pair' && report.state === 'succeeded') {
                console.log('Active ICE Candidate Pair:', report);
            }
        });

        const audioElement = document.getElementById('remoteAudio');
        if (audioElement) {
            console.log('Audio Element State:', {
                readyState: audioElement.readyState,
                paused: audioElement.paused,
                currentTime: audioElement.currentTime,
                srcObject: audioElement.srcObject ? 'present' : 'null',
                volume: audioElement.volume,
                muted: audioElement.muted
            });
        }
    };
    document.body.appendChild(button);
}

function addAudioDebugButton() {
    const button = document.createElement('button');
    button.textContent = 'Debug Audio';
    button.onclick = () => {
        const audioElement = document.getElementById('remoteAudio');
        console.log('Audio element state:', {
            exists: !!audioElement,
            srcObject: !!audioElement?.srcObject,
            paused: audioElement?.paused,
            muted: audioElement?.muted,
            volume: audioElement?.volume
        });
        
        if (peerConnection) {
            const receivers = peerConnection.getReceivers();
            receivers.forEach(receiver => {
                if (receiver.track.kind === 'audio') {
                    console.log('Audio receiver:', {
                        track: receiver.track,
                        transport: receiver.transport,
                        params: receiver.getParameters()
                    });
                }
            });
        }
    };
    document.body.appendChild(button);
}

// Initialize debug buttons
window.addEventListener('load', () => {
    addDebugButton();
    addAudioDebugButton();
}); 