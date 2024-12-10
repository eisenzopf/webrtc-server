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
        iceServers: [
            { urls: 'stun:stun.l.google.com:19302' }
        ]
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
        const candidateStr = event.candidate.candidate;
        console.log('New ICE candidate:', {
            candidate: candidateStr,
            type: event.candidate.type,
            protocol: candidateStr.split(' ')[2], // UDP/TCP
            address: candidateStr.split(' ')[4], // IP address
            port: candidateStr.split(' ')[5], // Port
            candidateType: candidateStr.split(' ')[7] // type (host/srflx/relay)
        });
        
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
    } else {
        console.log('ICE Candidate gathering complete');
    }
}

function handleIceConnectionStateChange() {
    const states = {
        iceConnectionState: peerConnection.iceConnectionState,
        signalingState: peerConnection.signalingState,
        connectionState: peerConnection.connectionState,
        iceGatheringState: peerConnection.iceGatheringState
    };
    
    console.log('ICE Connection State Change:', states);

    if (peerConnection.iceConnectionState === 'connected') {
        console.log('ICE Connection established');
        
        // Log the selected candidate pair
        peerConnection.getStats().then(stats => {
            stats.forEach(report => {
                if (report.type === 'candidate-pair' && report.selected) {
                    console.log('Selected candidate pair:', report);
                }
            });
        });

        // Monitor both streams
        const audioElement = document.getElementById('remoteAudio');
        if (audioElement && audioElement.srcObject) {
            console.log('Monitoring remote audio stream');
            monitorAudioState(audioElement.srcObject);
        } else {
            console.warn('No remote audio stream found');
        }
        
        if (localStream) {
            console.log('Monitoring local audio stream');
            monitorAudioState(localStream);
        } else {
            console.warn('No local stream found');
        }
    } else if (peerConnection.iceConnectionState === 'checking') {
        console.log('ICE Connection checking...');
    } else if (peerConnection.iceConnectionState === 'failed') {
        console.error('ICE Connection failed');
        updateCallStatus('failed');
    } else if (peerConnection.iceConnectionState === 'disconnected') {
        console.warn('ICE Connection disconnected');
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