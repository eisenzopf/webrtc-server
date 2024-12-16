// Add this import at the top of the file
import { updateStatus, updateCallStatus, updateButtonStates, resetAllPeerCheckboxes, setCurrentCallPeer } from './ui.js';
import { sendSignal } from './signaling.js';

// Export shared state and functions
export let peerConnection;
export let localStream;
export let remotePeerId = null;
export let enableVideo = false;
export let isInitiator = false;

// Add at the top of the file with other variables
let debugInterval = null;
let isNegotiating = false;
let dataChannel = null;

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

function createRemoteAudio() {
    const audioElement = document.createElement('audio');
    audioElement.id = 'remoteAudio';
    audioElement.autoplay = true;
    audioElement.controls = true;
    audioElement.volume = 1.0;
    audioElement.playsinline = true;
    document.body.appendChild(audioElement);
    console.log('Created new remote audio element with controls');
    return audioElement;
}

function handleICECandidate(event) {
    if (event.candidate) {
        console.log('Generated ICE candidate:', event.candidate);
        
        // Only send if we have a remote peer ID
        if (!remotePeerId) {
            console.warn('No remote peer ID set, cannot send ICE candidate');
            return;
        }

        // Ensure the candidate is properly stringified
        const candidateJson = JSON.stringify(event.candidate);
        
        sendSignal('IceCandidate', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peer: remotePeerId,
            candidate: candidateJson
        });
    }
}

async function handleNegotiationNeeded() {
    try {
        if (!peerConnection || peerConnection.signalingState !== "stable") {
            console.log("Skipping negotiation - not in stable state or no connection");
            return;
        }

        // Prevent multiple negotiations at once
        if (isNegotiating) {
            console.log("Already negotiating - skipping");
            return;
        }
        
        isNegotiating = true;
        
        try {
            console.log("Creating offer as negotiation is needed");
            const offer = await peerConnection.createOffer({
                // Add offerToReceiveAudio and offerToReceiveVideo options
                offerToReceiveAudio: true,
                offerToReceiveVideo: enableVideo
            });
            
            // Ensure consistent m-line ordering in SDP
            const modifiedSdp = offer.sdp.split('\r\n').map(line => {
                // Keep audio m-line first, followed by video
                if (line.startsWith('m=')) {
                    return line.startsWith('m=audio') ? line : null;
                }
                return line;
            }).filter(Boolean).join('\r\n');
            
            offer.sdp = modifiedSdp;
            
            // Double-check signaling state before setting local description
            if (peerConnection.signalingState === "stable") {
                // Wait for any pending operations
                await new Promise(resolve => setTimeout(resolve, 0));
                
                await peerConnection.setLocalDescription(offer);
                console.log("Set local description successfully");

                sendSignal('Offer', {
                    room_id: document.getElementById('roomId').value,
                    from_peer: document.getElementById('peerId').value,
                    to_peer: remotePeerId,
                    sdp: peerConnection.localDescription.sdp
                });
            } else {
                console.log("Signaling state changed during negotiation, aborting");
            }
        } finally {
            isNegotiating = false;
        }
    } catch (err) {
        console.error('Error during negotiation:', err);
        updateStatus('Failed to negotiate connection: ' + err.message, true);
        isNegotiating = false;
    }
}

export async function setupPeerConnection() {
    // Get TURN credentials from server
    const response = await fetch('/api/turn-credentials');
    const turnConfig = await response.json();
    
    const configuration = {
        iceServers: [{
            urls: `turn:${turnConfig.turn_server}:${turnConfig.turn_port}`,
            username: turnConfig.username,
            credential: turnConfig.password
        }],
        iceTransportPolicy: 'relay' // Force TURN usage for testing
    };

    peerConnection = new RTCPeerConnection(configuration);
    
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
        
        // Prevent negotiation during initial setup
        peerConnection.onnegotiationneeded = null;
        
        // Add tracks and wait for them to be fully processed
        const addTrackPromises = stream.getTracks().map(track => {
            console.log('Adding track to peer connection:', track.kind);
            return peerConnection.addTrack(track, stream);
        });
        
        await Promise.all(addTrackPromises);
        await new Promise(resolve => setTimeout(resolve, 200));

        // Add all event listeners
        peerConnection.ontrack = handleTrackEvent;
        peerConnection.onicecandidate = (event) => {
            if (event.candidate) {
                console.log('Generated ICE candidate:', event.candidate);
                sendSignal('IceCandidate', {
                    room_id: document.getElementById('roomId').value,
                    from_peer: document.getElementById('peerId').value,
                    to_peer: remotePeerId,
                    candidate: JSON.stringify(event.candidate)
                });
            }
        };
        peerConnection.onconnectionstatechange = () => {
            console.log('Connection state:', peerConnection.connectionState);
            handleConnectionStateChange();
        };
        peerConnection.oniceconnectionstatechange = () => {
            console.log('ICE connection state:', peerConnection.iceConnectionState);
        };
        peerConnection.onsignalingstatechange = () => {
            console.log('Signaling state:', peerConnection.signalingState);
        };

    } catch (err) {
        console.error('Error setting up peer connection:', err);
        throw err;
    }
}

// Add this function to check audio state
function debugAudioState() {
    const remoteAudio = document.getElementById('remoteAudio');
    if (remoteAudio) {
        console.log('Remote audio state:', {
            readyState: remoteAudio.readyState,
            paused: remoteAudio.paused,
            currentTime: remoteAudio.currentTime,
            srcObject: remoteAudio.srcObject ? {
                active: remoteAudio.srcObject.active,
                trackCount: remoteAudio.srcObject.getTracks().length,
                tracks: remoteAudio.srcObject.getTracks().map(t => ({
                    kind: t.kind,
                    enabled: t.enabled,
                    muted: t.muted,
                    readyState: t.readyState
                }))
            } : null
        });
    }
    
    if (peerConnection) {
        console.log('PeerConnection state:', {
            iceConnectionState: peerConnection.iceConnectionState,
            connectionState: peerConnection.connectionState,
            signalingState: peerConnection.signalingState
        });
    }
}

function handleTrackEvent(event) {
    console.log('Received remote track:', {
        kind: event.track.kind,
        id: event.track.id,
        enabled: event.track.enabled,
        muted: event.track.muted,
        readyState: event.track.readyState
    });
    
    const remoteStream = event.streams[0];
    if (remoteStream) {
        const audioElement = document.getElementById('remoteAudio') || createRemoteAudio();
        if (audioElement) {
            audioElement.srcObject = remoteStream;
            audioElement.play().catch(e => console.error('Audio play failed:', e));
            console.log('Set remote stream to audio element');
            
            // Add audio level monitoring
            const audioContext = new AudioContext();
            const source = audioContext.createMediaStreamSource(remoteStream);
            const analyser = audioContext.createAnalyser();
            source.connect(analyser);
            
            const dataArray = new Uint8Array(analyser.frequencyBinCount);
            
            setInterval(() => {
                analyser.getByteFrequencyData(dataArray);
                const average = dataArray.reduce((a, b) => a + b) / dataArray.length;
                console.log('Audio level:', average);
                updateAudioMeter(average);
            }, 100);
            
            // Monitor audio state
            setInterval(() => {
                console.log('Audio state:', {
                    time: audioElement.currentTime,
                    paused: audioElement.paused,
                    muted: audioElement.muted,
                    volume: audioElement.volume,
                    tracks: remoteStream.getTracks().map(t => ({
                        kind: t.kind,
                        enabled: t.enabled,
                        muted: t.muted,
                        readyState: t.readyState
                    }))
                });
            }, 5000);
        }
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
    
    const states = {
        connectionState: peerConnection.connectionState,
        iceConnectionState: peerConnection.iceConnectionState,
        iceGatheringState: peerConnection.iceGatheringState,
        signalingState: peerConnection.signalingState
    };
    
    console.log('Connection state changed:', states);

    // Log all tracks
    const receivers = peerConnection.getReceivers();
    receivers.forEach(receiver => {
        console.log(`${receiver.track.kind} receiver track:`, {
            enabled: receiver.track.enabled,
            muted: receiver.track.muted,
            readyState: receiver.track.readyState
        });
    });

    // Log connection state changes more verbosely
    if (states.connectionState === 'disconnected' || states.connectionState === 'failed' || states.connectionState === 'closed') {
        console.log('Connection entered terminal state:', states.connectionState);
    }
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

        // Setup new peer connection
        await setupPeerConnection();
        
        // Set initiator flag
        isInitiator = true;

        // Add a delay to ensure media tracks are fully added
        await new Promise(resolve => setTimeout(resolve, 500));

        // Verify tracks are added before proceeding
        const senders = peerConnection.getSenders();
        if (senders.length === 0) {
            throw new Error("No media tracks were added to the connection");
        }

        // Create and send offer
        const offer = await peerConnection.createOffer();
        await peerConnection.setLocalDescription(offer);
        console.log('Created and set local description (offer)');

        // Send call request with offer and SDP
        sendSignal('CallRequest', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peers: selectedPeers,
            sdp: offer.sdp
        });

        updateStatus('Initiating call...');
        updateCallStatus('connecting');
        updateButtonStates('connected', 'connecting');
    } catch (err) {
        console.error('Error starting call:', err);
        updateStatus('Failed to start call: ' + err.message, true);
        await cleanupExistingConnection();
        isInitiator = false;
        updateCallStatus('disconnected');
        updateButtonStates('connected', 'idle');
    }
}

export async function cleanupExistingConnection() {
    if (peerConnection) {
        console.log('Cleaning up existing peer connection...');
        try {
            // First remove all tracks
            const senders = peerConnection.getSenders();
            const removeTrackPromises = senders.map(sender => 
                peerConnection.removeTrack(sender)
            );
            await Promise.all(removeTrackPromises);

            // Stop all transceivers
            const transceivers = peerConnection.getTransceivers();
            transceivers.forEach(transceiver => {
                if (transceiver.stop) {
                    transceiver.stop();
                }
            });

            // Stop all tracks from senders
            senders.forEach(sender => {
                if (sender.track) {
                    sender.track.stop();
                }
            });

            // Close data channel if it exists
            if (dataChannel && dataChannel.readyState !== 'closed') {
                try {
                    dataChannel.close();
                } catch (err) {
                    console.warn('Error closing data channel:', err);
                }
                dataChannel = null;
            }
        
            // Stop all local tracks
            if (localStream) {
                localStream.getTracks().forEach(track => {
                    track.stop();
                });
                localStream = null;
            }

            // Clear video elements
            const localVideo = document.getElementById('localVideo');
            const remoteVideo = document.getElementById('remoteVideo');
            if (localVideo) localVideo.srcObject = null;
            if (remoteVideo) remoteVideo.srcObject = null;

            // Finally close the connection
            peerConnection.close();
            
            console.log('Peer connection cleanup completed');
            updateStatus('Connection cleaned up');

        } catch (err) {
            console.error('Error during connection cleanup:', err);
            throw new Error('Failed to cleanup WebRTC connection: ' + err.message);
        } finally {
            peerConnection = null;
        }
    }
}

export async function endCall() {
    console.log('Ending call...');
    try {
        // Clean up WebRTC connection
        await cleanupExistingConnection();
        
        // Additional cleanup for local stream
        if (localStream) {
            localStream.getTracks().forEach(track => track.stop());
            localStream = null;
        }
        
        // Reset video elements
        const localVideo = document.getElementById('localVideo');
        const remoteVideo = document.getElementById('remoteVideo');
        if (localVideo) localVideo.srcObject = null;
        if (remoteVideo) remoteVideo.srcObject = null;
        
        // Send end call signal to other peers
        sendSignal('EndCall', {
            room_id: document.getElementById('roomId').value,
            peer_id: document.getElementById('peerId').value
        });
        
        // Update UI
        updateCallStatus('disconnected');
        document.getElementById('startCallButton').disabled = false;
        document.getElementById('endCallButton').disabled = true;
        document.getElementById('audioStatus').textContent = 'Audio status: Not in call';
        
        console.log('Call ended successfully');
        updateButtonStates('connected', 'idle');
        
        // Reset current call peer and checkboxes
        setCurrentCallPeer(null);
        resetAllPeerCheckboxes();
        
    } catch (err) {
        console.error('Error ending call:', err);
        updateStatus('Error ending call: ' + err.message, true);
        // Still try to reset peer state even if there's an error
        setCurrentCallPeer(null);
        resetAllPeerCheckboxes();
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
        
        // Handle track updates sequentially to avoid race conditions
        for (const track of tracks) {
            console.log(`Processing ${track.kind} track for renegotiation`);
            const sender = senders.find(s => s.track && s.track.kind === track.kind);
            if (sender) {
                console.log(`Replacing existing ${track.kind} track`);
                await sender.replaceTrack(track);
                // Add small delay after each track replacement
                await new Promise(resolve => setTimeout(resolve, 100));
            } else {
                console.log(`Adding new ${track.kind} track`);
                peerConnection.addTrack(track, localStream);
                await new Promise(resolve => setTimeout(resolve, 100));
            }
        }

        // Add final delay before cleanup
        await new Promise(resolve => setTimeout(resolve, 200));

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

// Add these functions at the top level
function testAudioOutput() {
    const audioContext = new AudioContext();
    const oscillator = audioContext.createOscillator();
    const gainNode = audioContext.createGain();
    
    oscillator.connect(gainNode);
    gainNode.connect(audioContext.destination);
    
    oscillator.frequency.value = 440; // A4 note
    gainNode.gain.value = 0.1;       // 10% volume
    
    oscillator.start();
    setTimeout(() => oscillator.stop(), 1000);
}

// Export the test function
export { testAudioOutput };

// Add this function
function updateAudioMeter(level) {
    const meterFill = document.getElementById('audioMeterFill');
    if (meterFill) {
        // Convert level to percentage (assuming max level is 100)
        const percentage = Math.min(100, level * 2);
        meterFill.style.width = `${percentage}%`;
    }
}

export async function handleDisconnect(peer_id) {
    try {
        // Clean up the connection
        await cleanupExistingConnection();

        // Update UI
        updateStatus('Peer disconnected: ' + peer_id);
        document.getElementById('audioStatus').textContent = 'Audio status: Disconnected';

        // Reset connection state
        isConnected = false;
        remotePeerId = null;

        // Notify the application about the disconnection
        const event = new CustomEvent('peerDisconnected', {
            detail: { peerId: peer_id }
        });
        window.dispatchEvent(event);

    } catch (err) {
        console.error('Error handling disconnect:', err);
        updateStatus('Error handling disconnect: ' + err.message);
    }
}

// Add event listener for connection state changes
export function setupConnectionStateHandler(pc) {
    if (!pc) return;

    pc.onconnectionstatechange = () => {
        console.log('Connection state changed:', pc.connectionState);
        
        switch (pc.connectionState) {
            case 'disconnected':
            case 'failed':
            case 'closed':
                handleDisconnect(remotePeerId);
                break;
            case 'connected':
                updateStatus('Connected to peer');
                break;
            case 'connecting':
                updateStatus('Connecting...');
                break;
        }
    };
}