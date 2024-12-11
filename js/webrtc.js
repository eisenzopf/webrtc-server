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
        iceServers: [{
            urls: 'stun:127.0.0.1:3478'
        }],
        iceTransportPolicy: 'all',
        bundlePolicy: 'max-bundle',
        rtcpMuxPolicy: 'require',
        iceCandidatePoolSize: 0
    });

    // Add connection monitoring
    peerConnection.onconnectionstatechange = () => {
        console.log(`Connection state changed: ${peerConnection.connectionState}`);
        if (peerConnection.connectionState === 'connected') {
            console.log('Peer connection successfully established');
            updateCallStatus('connected', remotePeerId);
        } else if (peerConnection.connectionState === 'failed') {
            console.error('Connection failed - attempting ICE restart');
            peerConnection.restartIce();
        }
    };

    peerConnection.oniceconnectionstatechange = () => {
        console.log(`ICE connection state: ${peerConnection.iceConnectionState}`);
        if (peerConnection.iceConnectionState === 'failed') {
            console.error('ICE Connection failed - creating new offer');
            createAndSendOffer();
        }
    };

    // Existing event handlers...
    peerConnection.ontrack = handleTrackEvent;
    peerConnection.onicecandidate = handleIceCandidate;
    peerConnection.onsignalingstatechange = () => {
        console.log('Signaling State Change:', peerConnection.signalingState);
    };

    // Enhanced ICE gathering state monitoring
    peerConnection.onicegatheringstatechange = () => {
        console.log('ICE gathering state:', peerConnection.iceGatheringState);
        if (peerConnection.iceGatheringState === 'complete') {
            const candidates = extractIceCandidates(peerConnection.localDescription.sdp);
            console.log('All ICE candidates:', candidates);
        }
    };

    peerConnection.onnegotiationneeded = async () => {
        console.log('Negotiation needed');
        try {
            await startCall();  // Only if not already in a call
        } catch (err) {
            console.error('Error during negotiation:', err);
        }
    };

    console.log("Adding local tracks to connection");
    localStream.getTracks().forEach(track => {
        peerConnection.addTrack(track, localStream);
    });

    // Handle incoming tracks
    peerConnection.ontrack = (event) => {
        console.log('Received remote track:', event.track.kind);
        const audioElement = new Audio();
        audioElement.srcObject = event.streams[0];
        audioElement.play().catch(err => console.error('Error playing audio:', err));
    };

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
        const candidate = event.candidate;
        
        // Log detailed candidate information
        console.log('ICE candidate:', {
            type: candidate.type,
            protocol: candidate.protocol,
            address: candidate.address,
            port: candidate.port,
            priority: candidate.priority,
            foundation: candidate.foundation,
            component: candidate.component,
            relatedAddress: candidate.relatedAddress,
            relatedPort: candidate.relatedPort,
            tcpType: candidate.tcpType,
            usernameFragment: candidate.usernameFragment
        });

        sendSignal('IceCandidate', {
            room_id: document.getElementById('roomId').value,
            candidate: {
                candidate: candidate.candidate,
                sdpMid: candidate.sdpMid,
                sdpMLineIndex: candidate.sdpMLineIndex,
                usernameFragment: candidate.usernameFragment
            },
            from_peer: document.getElementById('peerId').value,
            to_peer: remotePeerId
        });
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

// Add this helper function to extract ICE candidates from SDP
function extractIceCandidates(sdp) {
    const candidates = [];
    const lines = sdp.split('\r\n');
    lines.forEach(line => {
        if (line.indexOf('a=candidate:') === 0) {
            const parts = line.split(' ');
            candidates.push({
                foundation: parts[0].split(':')[1],
                component: parts[1],
                protocol: parts[2],
                priority: parts[3],
                ip: parts[4],
                port: parts[5],
                type: parts[7]
            });
        }
    });
    return candidates;
} 