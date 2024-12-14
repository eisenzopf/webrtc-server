// Add this import at the top of the file
import { updateStatus } from './ui.js';
import { sendSignal } from './signaling.js';

// Export shared state and functions
export let peerConnection;
export let localStream;
export let remotePeerId = null;
export let enableVideo = false;

export async function getIceServers() {
    try {
        const serverAddress = window.location.hostname;
        const serverPort = window.location.port || '8080';
        const response = await fetch(`http://${serverAddress}:${serverPort}/api/turn-credentials`);
        if (!response.ok) {
            throw new Error('Failed to fetch TURN credentials');
        }
        const credentials = await response.json();
        
        const connectionType = document.getElementById('connectionType').value;
        
        return {
            iceServers: [{
                urls: [
                    `stun:${credentials.stun_server}:${credentials.stun_port}`,
                    `turn:${credentials.turn_server}:${credentials.turn_port}`
                ],
                username: credentials.username,
                credential: credentials.password
            }],
            iceTransportPolicy: connectionType === 'relay' ? 'relay' : 'all',
            iceCandidatePoolSize: 10,
            bundlePolicy: 'max-bundle',
            rtcpMuxPolicy: 'require'
        };
    } catch (err) {
        console.error('Error getting ICE servers:', err);
        throw new Error('Failed to fetch TURN credentials');
    }
}

export function handleTrack(event) {
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

export async function setupPeerConnection() {
    const config = await getIceServers();
    peerConnection = new RTCPeerConnection(config);
    
    try {
        const stream = await navigator.mediaDevices.getUserMedia({
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            },
            video: false
        });
        
        setLocalStream(stream);
        stream.getTracks().forEach(track => {
            console.log('Adding track to peer connection:', track.kind);
            peerConnection.addTrack(track, stream);
        });
    } catch (err) {
        console.error('Error getting user media:', err);
        updateStatus('Failed to access microphone: ' + err.message, true);
        throw err; // Re-throw to handle in caller
    }

    // Add ICE candidate handling
    peerConnection.onicecandidate = (event) => {
        if (event.candidate) {
            sendSignal('IceCandidate', {
                room_id: document.getElementById('roomId').value,
                from_peer: document.getElementById('peerId').value,
                to_peer: remotePeerId,
                candidate: {
                    candidate: event.candidate.candidate,
                    sdpMid: event.candidate.sdpMid,
                    sdpMLineIndex: event.candidate.sdpMLineIndex
                }
            });
        }
    };

    // Log ICE connection state changes
    peerConnection.oniceconnectionstatechange = () => {
        console.log('ICE connection state:', peerConnection.iceConnectionState);
    };

    return peerConnection;
}

function handleTrackEvent(event) {
    console.log('Received remote track:', event.track.kind);
    const remoteStream = event.streams[0];
    
    if (remoteStream) {
        const audioElement = document.getElementById('remoteAudio');
        if (audioElement) {
            audioElement.srcObject = remoteStream;
            console.log('Set remote stream to audio element');
            
            // Monitor audio levels
            monitorAudioState(remoteStream);
        } else {
            console.warn('No remote audio element found');
        }
    } else {
        console.warn('No remote stream in track event');
    }
}

function handleICECandidate(event) {
    if (event.candidate) {
        console.log('Generated ICE candidate:', event.candidate);
        
        sendSignal('IceCandidate', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peer: getRemotePeerId()
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

export async function startCall() {
    try {
        const selectedPeers = Array.from(document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked'))
            .map(cb => cb.value);
        
        if (selectedPeers.length === 0) {
            updateStatus('Please select at least one peer to call', true);
            return;
        }

        // Set the remote peer ID first
        setRemotePeerId(selectedPeers[0]);

        // Clean up any existing connections
        await cleanupExistingConnection();

        // Setup new peer connection (this will also get media stream and add tracks)
        await setupPeerConnection();

        // Create and send offer
        const offer = await peerConnection.createOffer();
        await peerConnection.setLocalDescription(offer);

        // Send call request with offer and SDP
        sendSignal('CallRequest', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peers: selectedPeers,
            sdp: offer.sdp
        });

        updateStatus('Initiating server-relayed call...');
    } catch (err) {
        console.error('Error starting call:', err);
        updateStatus('Failed to start call: ' + err.message, true);
        await cleanupExistingConnection();
    }
}

async function cleanupExistingConnection() {
    if (peerConnection) {
        const senders = peerConnection.getSenders();
        const promises = senders.map(sender => 
            peerConnection.removeTrack(sender)
        );
        await Promise.all(promises);
        peerConnection.close();
        peerConnection = null;
    }

    if (localStream) {
        localStream.getTracks().forEach(track => {
            track.stop();
        });
        localStream = null;
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

export async function checkMediaDevices() {
    try {
        // First check if mediaDevices API is available
        if (!navigator.mediaDevices) {
            console.warn('MediaDevices API not available');
            const mediaControls = document.getElementById('mediaControls');
            if (mediaControls) {
                mediaControls.style.display = 'none';
            }
            
            // Check common issues
            if (window.location.protocol === 'file:') {
                throw new Error('Media devices cannot be accessed when loading from a file. Please serve the page through a web server.');
            } else if (window.location.protocol !== 'https:' && window.location.hostname !== 'localhost') {
                throw new Error('Media devices require a secure connection (HTTPS) or localhost');
            } else {
                throw new Error('MediaDevices API is not supported in this browser');
            }
        }

        const devices = await navigator.mediaDevices.enumerateDevices();
        const hasCamera = devices.some(device => device.kind === 'videoinput');
        
        const mediaControls = document.getElementById('mediaControls');
        const enableVideoCheckbox = document.getElementById('enableVideo');
        
        if (mediaControls) {
            mediaControls.style.display = 'block';
        }
        
        if (enableVideoCheckbox) {
            enableVideoCheckbox.checked = false;
            enableVideoCheckbox.onchange = (e) => {
                const enableVideo = e.target.checked;
                if (window.peerConnection && window.peerConnection.connectionState === 'connected') {
                    // Ensure renegotiateMedia is defined
                    if (typeof window.renegotiateMedia === 'function') {
                        window.renegotiateMedia();
                    }
                }
            };
        }

        return hasCamera;
    } catch (err) {
        console.error('Error checking media devices:', err);
        // Show a more user-friendly error message in the UI
        const audioStatus = document.getElementById('audioStatus');
        if (audioStatus) {
            audioStatus.textContent = `Media Error: ${err.message}`;
            audioStatus.style.color = 'red';
        }
        throw err;
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

function debugIceFailure() {
    if (!peerConnection) return;
    
    peerConnection.getStats().then(stats => {
        let iceCandidates = {
            local: [],
            remote: [],
            pairs: []
        };

        stats.forEach(report => {
            if (report.type === 'local-candidate') {
                iceCandidates.local.push({
                    type: report.candidateType,
                    protocol: report.protocol,
                    address: report.address,
                    port: report.port
                });
            } else if (report.type === 'remote-candidate') {
                iceCandidates.remote.push({
                    type: report.candidateType,
                    protocol: report.protocol,
                    address: report.address,
                    port: report.port
                });
            } else if (report.type === 'candidate-pair') {
                iceCandidates.pairs.push({
                    state: report.state,
                    nominated: report.nominated,
                    selected: report.selected,
                    localCandidateId: report.localCandidateId,
                    remoteCandidateId: report.remoteCandidateId
                });
            }
        });

        console.log('ICE Candidates Debug:', iceCandidates);
    });
}

export function resetPeerConnection() {
    peerConnection = null;
}

export function resetLocalStream() {
    localStream = null;
}

export function resetRemotePeerId() {
    remotePeerId = null;
}

export function setRemotePeerId(peerId) {
    remotePeerId = peerId;
}

export function setLocalStream(stream) {
    localStream = stream;
}