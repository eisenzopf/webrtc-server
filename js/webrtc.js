let peerConnection;
let localStream;
let remotePeerId = null;

async function setupPeerConnection() {
    if (!localStream) {
        console.log("Getting user media");
        localStream = await navigator.mediaDevices.getUserMedia({ 
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            }
        });
    }

    console.log("Creating new RTCPeerConnection");
    peerConnection = new RTCPeerConnection({
        sdpSemantics: 'unified-plan',
        iceServers: [{
            urls: [
                'stun:stun.l.google.com:19302',
                'stun:stun1.l.google.com:19302'
            ]
        }]
    });

    peerConnection.ontrack = handleTrackEvent;
    peerConnection.onicecandidate = handleIceCandidate;
    peerConnection.oniceconnectionstatechange = handleIceConnectionStateChange;
    peerConnection.onsignalingstatechange = () => {
        console.log('Signaling State Change:', peerConnection.signalingState);
    };
    peerConnection.onconnectionstatechange = handleConnectionStateChange;

    console.log("Adding local tracks to connection");
    localStream.getTracks().forEach(track => {
        track.enabled = true;
        peerConnection.addTrack(track, localStream);
        console.log('Local track added:', {
            kind: track.kind,
            enabled: track.enabled,
            muted: track.muted,
            id: track.id
        });
    });

    return peerConnection;
}

function handleTrackEvent(event) {
    console.log('Track received:', {
        kind: event.track.kind,
        readyState: event.track.readyState,
        enabled: event.track.enabled,
        muted: event.track.muted,
        id: event.track.id
    });

    if (event.track.kind === 'audio') {
        let audioElement = document.getElementById('remoteAudio');
        if (!audioElement) {
            audioElement = document.createElement('audio');
            audioElement.id = 'remoteAudio';
            audioElement.autoplay = true;
            audioElement.playsInline = true;
            audioElement.controls = true;
            audioElement.volume = 1.0;
            document.body.appendChild(audioElement);
        }

        const stream = new MediaStream([event.track]);
        audioElement.srcObject = stream;
        
        audioElement.play()
            .then(() => console.log('Initial audio playback started'))
            .catch(e => console.error('Initial play failed:', e));

        event.track.onunmute = () => {
            audioElement.play()
                .then(() => console.log('Audio playback started after unmute'))
                .catch(e => console.error('Play after unmute failed:', e));
        };
    }
}

function handleIceCandidate(event) {
    if (event.candidate) {
        console.log('Sending ICE candidate:', event.candidate);
        sendSignal('IceCandidate', {
            room_id: document.getElementById('roomId').value,
            candidate: {
                candidate: event.candidate.candidate,
                sdpMid: event.candidate.sdpMid,
                sdpMLineIndex: event.candidate.sdpMLineIndex
            },
            from_peer: document.getElementById('peerId').value,
            to_peer: remotePeerId
        });
    }
}

function handleIceConnectionStateChange() {
    console.log('ICE Connection State Change:', {
        iceConnectionState: peerConnection.iceConnectionState,
        signalingState: peerConnection.signalingState,
        connectionState: peerConnection.connectionState,
        iceGatheringState: peerConnection.iceGatheringState
    });

    if (peerConnection.iceConnectionState === 'connected') {
        console.log('ICE Connection established');
        const audioElement = document.getElementById('remoteAudio');
        if (audioElement && audioElement.srcObject) {
            monitorAudioState(audioElement.srcObject);
        }
        if (localStream) {
            monitorAudioState(localStream);
        }
    } else if (peerConnection.iceConnectionState === 'failed') {
        console.error('ICE Connection failed');
        updateCallStatus('failed');
    }
}

function handleConnectionStateChange() {
    const state = {
        connectionState: peerConnection.connectionState,
        iceConnectionState: peerConnection.iceConnectionState,
        signalingState: peerConnection.signalingState,
        iceGatheringState: peerConnection.iceGatheringState
    };
    console.log('Connection State Change:', state);
    
    if (peerConnection.connectionState === 'connected') {
        updateCallStatus('connected', remotePeerId);
    } else if (peerConnection.connectionState === 'failed' || 
               peerConnection.connectionState === 'disconnected') {
        updateCallStatus(peerConnection.connectionState);
    }
}

async function startCall() {
    const selectedPeers = Array.from(document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked'))
        .map(cb => cb.value);
    
    if (selectedPeers.length === 0) {
        updateStatus('Please select at least one peer to call', true);
        return;
    }

    sendSignal('CallRequest', {
        room_id: document.getElementById('roomId').value,
        from_peer: document.getElementById('peerId').value,
        to_peers: selectedPeers
    });
    
    updateStatus(`Calling ${selectedPeers.length} peer(s)...`);
}

async function endCall() {
    try {
        if (peerConnection) {
            peerConnection.close();
            peerConnection = null;

            if (localStream) {
                localStream.getTracks().forEach(track => track.stop());
                localStream = null;
            }

            sendSignal('EndCall', {
                room_id: document.getElementById('roomId').value,
                peer_id: document.getElementById('peerId').value
            });

            updateStatus('Call ended');
            document.getElementById('audioStatus').textContent = 'Audio status: Not in call';
        } else {
            updateStatus('No active call to end');
        }
    } catch (err) {
        console.error('Error ending call:', err);
        updateStatus('Error ending call: ' + err.message, true);
    }
} 