let currentCallPeer = null;

export function setCurrentCallPeer(peerId) {
    currentCallPeer = peerId;
}

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
        timestamp: new Date().toISOString(),
        peerConnection: peerConnection ? {
            iceConnectionState: peerConnection.iceConnectionState,
            connectionState: peerConnection.connectionState,
            signalingState: peerConnection.signalingState,
            iceGatheringState: peerConnection.iceGatheringState
        } : null
    });
}

export function updateCallStatus(state, peer = null) {
    const statusMsg = peer ? `${state} with ${peer}` : state;
    updateStatus(statusMsg);
    
    let callState = 'idle';
    
    if (state === 'connected') {
        document.getElementById('audioStatus').textContent = 'Audio status: Connected';
        document.getElementById('connectionStatus').className = 'status success';
        callState = 'incall';
    } else if (state === 'disconnected' || state === 'failed') {
        document.getElementById('audioStatus').textContent = 'Audio status: Not connected';
        document.getElementById('connectionStatus').className = 'status error';
        callState = 'idle';
    } else if (state === 'ready') {
        document.getElementById('audioStatus').textContent = 'Audio status: Ready';
        document.getElementById('connectionStatus').className = 'status';
        callState = 'idle';
    }
    
    updateButtonStates('connected', callState);
}

export function handlePeerListMessage(message) {
    console.log("Handling peer list update:", message);
    const currentPeerId = document.getElementById('peerId').value;
    const otherPeers = message.peers.filter(p => p !== currentPeerId);
    console.log("Filtered peers (excluding self):", otherPeers);
    
    // Store currently checked peers before updating list
    const previouslyCheckedPeers = Array.from(document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked'))
        .map(cb => cb.value);
    
    const peerListDiv = document.getElementById('selectablePeerList');
    peerListDiv.innerHTML = otherPeers.map(peerId => `
        <div class="peer-item">
            <input type="checkbox" id="peer_${peerId}" value="${peerId}">
            <label for="peer_${peerId}">${peerId}</label>
        </div>
    `).join('');
    
    // Add change event listeners to checkboxes and restore states
    const checkboxes = peerListDiv.querySelectorAll('input[type="checkbox"]');
    checkboxes.forEach(checkbox => {
        checkbox.addEventListener('change', () => {
            const selectedPeers = document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked');
            updateButtonStates('connected', 'idle');
        });
        
        // If this peer is the one we're in a call with, disable and check it
        if (currentCallPeer && checkbox.value === currentCallPeer) {
            checkbox.checked = true;
            checkbox.disabled = true;
        } else if (previouslyCheckedPeers.includes(checkbox.value) && !currentCallPeer) {
            // Restore previous check state only if not in a call
            checkbox.checked = true;
        }
    });
    
    console.log("Updated peer list HTML with currentCallPeer:", currentCallPeer);
}

export function showCallAlert(caller) {
    return new Promise((resolve) => {
        const result = window.confirm(`Incoming call from ${caller}. Accept?`);
        resolve(result);
    });
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

// Add this new function
export function initializeButtonStates() {
    updateButtonStates('disconnected', 'idle');
}

// Modify the existing load event listener
window.addEventListener('load', () => {
    addDebugButton();
    addAudioDebugButton();
    initializeButtonStates();
});

// Add a new function to handle disconnect state
export function handleDisconnectState() {
    updateCallStatus('disconnected');
    document.getElementById('connectButton').disabled = false;
    document.getElementById('startCallButton').disabled = true;
    document.getElementById('endCallButton').disabled = true;
    document.getElementById('disconnectButton').disabled = true;
}

export function updateButtonStates(connectionState, callState) {
    const connectButton = document.getElementById('connectButton');
    const startCallButton = document.getElementById('startCallButton');
    const endCallButton = document.getElementById('endCallButton');
    const disconnectButton = document.getElementById('disconnectButton');

    // Default all buttons to disabled
    startCallButton.disabled = true;
    endCallButton.disabled = true;
    disconnectButton.disabled = true;

    switch (connectionState) {
        case 'connected':
            connectButton.disabled = true;
            disconnectButton.disabled = false;
            
            // Enable call buttons only if peers are selected
            const selectedPeers = document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked');
            startCallButton.disabled = selectedPeers.length === 0;
            
            // Enable end call button only if in a call
            endCallButton.disabled = callState !== 'incall';
            break;
            
        case 'disconnected':
            connectButton.disabled = false;
            break;
            
        default:
            connectButton.disabled = false;
    }
}

// Add these new functions
export function updatePeerCheckboxState(peerId, checked, disabled) {
    const checkbox = document.querySelector(`#peer_${peerId}`);
    if (checkbox) {
        checkbox.checked = checked;
        checkbox.disabled = disabled;
    }
}

export function resetAllPeerCheckboxes() {
    const checkboxes = document.querySelectorAll('#selectablePeerList input[type="checkbox"]');
    checkboxes.forEach(checkbox => {
        checkbox.disabled = false;
    });
    currentCallPeer = null;
} 