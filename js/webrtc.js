// Add this import at the top of the file
import { updateStatus } from './ui.js';
import { sendSignal } from './signaling.js';

// Export shared state and functions
export let peerConnection;
export let localStream;
export let remotePeerId = null;
export let enableVideo = false;

export async function getIceServers() {
    const stunServer = document.getElementById('stunServer').value;
    const stunPort = document.getElementById('stunPort').value;
    const username = document.getElementById('turnUsername').value;
    const password = document.getElementById('turnPassword').value;
    const connectionType = document.getElementById('connectionType').value;

    const iceServers = [{
        urls: [
            `stun:${stunServer}:${stunPort}`,
            `turn:${stunServer}:${stunPort}`
        ],
        username: username,
        credential: password
    }];

    return {
        iceServers,
        iceTransportPolicy: connectionType === 'relay' ? 'relay' : 'all',
        iceCandidatePoolSize: 10,
        bundlePolicy: 'max-bundle',
        rtcpMuxPolicy: 'require'
    };
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
    if (peerConnection) {
        console.log('Peer connection already exists');
        return;
    }

    const config = await getIceServers();
    if (!config || !config.iceServers) {
        throw new Error('Failed to get ICE server configuration');
    }
    
    console.log('Using connection type:', config.type || 'default');
    peerConnection = new RTCPeerConnection(config.iceServers);
    
    // Add event handlers
    peerConnection.onicecandidate = (event) => {
        if (event.candidate) {
            console.log('Generated ICE candidate:', event.candidate);
            if (!remotePeerId) {
                console.warn('No remote peer ID set for ICE candidate');
                return;
            }

            const candidateInit = {
                candidate: event.candidate.candidate,
                sdpMid: event.candidate.sdpMid,
                sdpMLineIndex: event.candidate.sdpMLineIndex,
                usernameFragment: event.candidate.usernameFragment
            };

            sendSignal('IceCandidate', {
                room_id: document.getElementById('roomId').value,
                candidate: JSON.stringify(candidateInit),
                from_peer: document.getElementById('peerId').value,
                to_peer: remotePeerId
            });
        }
    };

    peerConnection.onicegatheringstatechange = () => {
        console.log('ICE gathering state:', peerConnection.iceGatheringState);
        if (peerConnection.iceGatheringState === 'complete') {
            console.log('Final ICE candidates:', peerConnection.localDescription.sdp);
        }
    };

    peerConnection.ontrack = handleTrack;
    peerConnection.oniceconnectionstatechange = () => {
        console.log('ICE connection state:', peerConnection.iceConnectionState);
    };
    
    // Add local stream tracks to the connection
    if (localStream) {
        localStream.getTracks().forEach(track => {
            console.log('Adding track to peer connection:', track.kind);
            peerConnection.addTrack(track, localStream);
        });
    }
    
    return peerConnection;
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

export async function startCall() {
    try {
        const selectedPeers = Array.from(document.querySelectorAll('#selectablePeerList input[type="checkbox"]:checked'))
            .map(cb => cb.value);
        
        if (selectedPeers.length === 0) {
            updateStatus('Please select at least one peer to call', true);
            return;
        }

        // Clean up any existing peer connection
        if (peerConnection) {
            const senders = peerConnection.getSenders();
            const promises = senders.map(sender => 
                peerConnection.removeTrack(sender)
            );
            await Promise.all(promises);
            peerConnection.close();
            peerConnection = null;
        }

        // Clean up any existing local stream
        if (localStream) {
            localStream.getTracks().forEach(track => {
                track.stop();
            });
            localStream = null;
        }

        // Get media stream first
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

        // Setup new peer connection
        await setupPeerConnection();
        
        // Add tracks to the new peer connection
        localStream.getTracks().forEach(track => {
            console.log('Adding track to peer connection:', track.kind);
            const existingSender = peerConnection.getSenders().find(s => 
                s.track && s.track.kind === track.kind
            );
            if (existingSender) {
                existingSender.replaceTrack(track);
            } else {
                peerConnection.addTrack(track, localStream);
            }
        });

        // Send CallRequest to initiate server-relayed call
        sendSignal('CallRequest', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peers: selectedPeers
        });

        updateStatus('Initiating server-relayed call...');
    } catch (err) {
        console.error('Error starting call:', err);
        updateStatus('Failed to start call: ' + err.message, true);
        
        // Clean up on error
        if (localStream) {
            localStream.getTracks().forEach(track => track.stop());
            localStream = null;
        }
        if (peerConnection) {
            peerConnection.close();
            peerConnection = null;
        }
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