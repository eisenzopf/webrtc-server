let peerConnection;
let localStream;
let remotePeerId = null;
let hasCamera = false;
let enableVideo = false;

function handleTrackEvent(event) {
    console.log('Received track from server:', event.track.kind);
    
    if (event.track.kind === 'audio') {
        const audioElement = document.getElementById('remoteAudio') || createRemoteAudio();
        if (!audioElement.srcObject) {
            audioElement.srcObject = new MediaStream();
        }
        audioElement.srcObject.addTrack(event.track);
    } else if (event.track.kind === 'video') {
        const videoElement = document.getElementById('remoteVideo');
        if (!videoElement.srcObject) {
            videoElement.srcObject = new MediaStream();
        }
        videoElement.srcObject.addTrack(event.track);
        videoElement.style.display = 'block';
        console.log('Added video track to remote video element');
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
    if (!peerConnection) return;
    
    console.log('Connection state changed:', {
        connectionState: peerConnection.connectionState,
        iceConnectionState: peerConnection.iceConnectionState,
        iceGatheringState: peerConnection.iceGatheringState,
        signalingState: peerConnection.signalingState
    });

    // Check all tracks
    const receivers = peerConnection.getReceivers();
    receivers.forEach(receiver => {
        console.log(`${receiver.track.kind} receiver track:`, {
            enabled: receiver.track.enabled,
            muted: receiver.track.muted,
            readyState: receiver.track.readyState
        });
    });
}

async function startCall() {
    try {
        // Single peer connection to the server
        await setupPeerConnection();
        
        const selectedPeers = Array.from(document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked'))
            .map(cb => cb.value);
        
        if (selectedPeers.length === 0) {
            updateStatus('Please select at least one peer to call', true);
            return;
        }

        // Send call request to server
        sendSignal('CallRequest', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peers: selectedPeers
        });
        
        updateStatus(`Joining call with ${selectedPeers.length} peer(s)...`);
    } catch (err) {
        console.error('Error starting call:', err);
        updateStatus('Failed to start call: ' + err.message, true);
    }
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

async function checkMediaDevices() {
    try {
        const devices = await navigator.mediaDevices.enumerateDevices();
        hasCamera = devices.some(device => device.kind === 'videoinput');
        
        const mediaControls = document.getElementById('mediaControls');
        const enableVideoCheckbox = document.getElementById('enableVideo');
        
        if (hasCamera) {
            mediaControls.style.display = 'block';
            enableVideoCheckbox.checked = false;
            enableVideoCheckbox.onchange = (e) => {
                enableVideo = e.target.checked;
                // If we're already in a call, we need to renegotiate
                if (peerConnection && peerConnection.connectionState === 'connected') {
                    renegotiateMedia();
                }
            };
        } else {
            mediaControls.style.display = 'none';
        }
    } catch (err) {
        console.error('Error checking media devices:', err);
    }
}

async function renegotiateMedia() {
    try {
        // Stop all current tracks
        if (localStream) {
            localStream.getTracks().forEach(track => {
                track.stop();
                localStream.removeTrack(track);
            });
        }

        // Get new media stream with updated constraints
        const constraints = {
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            },
            video: enableVideo ? {
                width: { ideal: 1280 },
                height: { ideal: 720 },
                frameRate: { ideal: 30 }
            } : false
        };

        console.log('Requesting media with constraints:', constraints);
        localStream = await navigator.mediaDevices.getUserMedia(constraints);
        
        // Replace all tracks in the peer connection
        const senders = peerConnection.getSenders();
        const tracks = localStream.getTracks();
        
        for (const track of tracks) {
            console.log(`Processing ${track.kind} track for renegotiation`);
            const sender = senders.find(s => s.track && s.track.kind === track.kind);
            if (sender) {
                console.log(`Replacing existing ${track.kind} track`);
                await sender.replaceTrack(track);
            } else {
                console.log(`Adding new ${track.kind} track`);
                peerConnection.addTrack(track, localStream);
            }
        }

        // Remove any senders that no longer have corresponding tracks
        for (const sender of senders) {
            if (!localStream.getTracks().some(track => track.kind === sender.track?.kind)) {
                console.log(`Removing ${sender.track?.kind} sender`);
                peerConnection.removeTrack(sender);
            }
        }

        updateVideoUI();
    } catch (err) {
        console.error('Error renegotiating media:', err);
        updateStatus('Failed to update media settings', true);
    }
}

function updateVideoUI() {
    const localVideo = document.getElementById('localVideo');
    const remoteVideo = document.getElementById('remoteVideo');
    
    if (enableVideo) {
        localVideo.style.display = 'block';
        remoteVideo.style.display = 'block';
        if (localStream) {
            localVideo.srcObject = localStream;
        }
    } else {
        localVideo.style.display = 'none';
        remoteVideo.style.display = 'none';
        localVideo.srcObject = null;
        remoteVideo.srcObject = null;
    }
}

async function setupPeerConnection() {
    try {
        // Get STUN server configuration from input fields
        const stunServer = document.getElementById('stunServer').value || '127.0.0.1';
        const stunPort = document.getElementById('stunPort').value || '3478';
        const stunUrl = `stun:${stunServer}:${stunPort}`;

        const configuration = {
            iceServers: [
                { 
                    urls: stunUrl,
                    username: "",
                    credential: ""
                }
            ],
            iceTransportPolicy: 'relay',
            bundlePolicy: 'balanced',
            rtcpMuxPolicy: 'require',
            iceCandidatePoolSize: 10
        };

        // Only create a new connection if one doesn't exist or is closed
        if (peerConnection && peerConnection.connectionState !== 'closed') {
            console.log("Peer connection already exists");
            return peerConnection;
        }

        // Add specific audio constraints
        const constraints = {
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            },
            video: enableVideo ? {
                width: { ideal: 1280 },
                height: { ideal: 720 },
                frameRate: { ideal: 30 }
            } : false
        };

        // Get user media with logging
        console.log('Requesting user media with constraints:', constraints);
        localStream = await navigator.mediaDevices.getUserMedia(constraints);
        
        // Verify audio tracks exist
        const audioTracks = localStream.getAudioTracks();
        console.log('Local audio tracks:', audioTracks);
        
        if (audioTracks.length === 0) {
            throw new Error('No audio track found in local stream');
        }

        updateVideoUI();

        console.log("Creating new RTCPeerConnection with configuration:", configuration);
        peerConnection = new RTCPeerConnection(configuration);

        // Add connection monitoring
        peerConnection.onconnectionstatechange = handleConnectionStateChange;
        peerConnection.oniceconnectionstatechange = handleIceConnectionStateChange;
        peerConnection.ontrack = handleTrackEvent;
        peerConnection.onicecandidate = handleIceCandidate;

        // Add local stream tracks to the connection
        localStream.getTracks().forEach(track => {
            console.log(`Adding track to peer connection: ${track.kind}`);
            peerConnection.addTrack(track, localStream);
        });

        return peerConnection;
    } catch (err) {
        console.error('Error setting up peer connection:', err);
        updateStatus('Failed to access microphone. Please grant permissions and try again.', true);
        throw err;
    }
}