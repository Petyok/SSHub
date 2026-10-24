// Camera scanning: video frames -> jsQR -> fountain frames -> one document. The terminal loops
// its frames forever, so a missed frame comes round again; parity repairs one loss per group.
import jsQR from '../vendor/jsqr-1.4.0/jsQR.cjs';
import { FOUNTAIN_HEADER, FountainDecoder } from './quorum.mjs';

const MAX_SIDE = 720;

export class DocumentCollector {
  constructor() {
    this.reset();
  }
  reset() {
    this.dec = new FountainDecoder();
    this.frames = 0;
    this.errors = 0;
  }
  // Returns the document bytes when complete, else null. A frame of another document is ignored.
  push(frame) {
    this.frames++;
    try {
      const r = this.dec.push(frame);
      if (r === 'other') return null;
      const doc = this.dec.finish();
      if (doc) return doc;
    } catch {
      this.errors++;
      this.dec = new FountainDecoder();
    }
    return null;
  }
  progress() {
    const h = this.dec.head;
    if (!h) return { have: 0, total: 0 };
    return { have: this.dec.blocks.filter((b) => b !== null).length, total: h.k };
  }
}

// Opens the rear camera into `video` and calls onFrame(bytes) for every QR code read.
// Returns { stop() }. Rejects when the camera is unavailable or denied.
export async function startCamera(video, onFrame, { facing = 'environment' } = {}) {
  if (!globalThis.isSecureContext || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
    throw new Error('camera needs a secure page (https:// or localhost; file:// works on Android Chrome)');
  }
  const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: { ideal: facing }, width: { ideal: 1280 } }, audio: false });
  video.srcObject = stream;
  video.setAttribute('playsinline', '');
  await video.play();
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  let alive = true;
  const tick = () => {
    if (!alive) return;
    if (video.videoWidth > 0) {
      const k = Math.min(1, MAX_SIDE / Math.max(video.videoWidth, video.videoHeight));
      canvas.width = Math.round(video.videoWidth * k);
      canvas.height = Math.round(video.videoHeight * k);
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const r = jsQR(img.data, img.width, img.height, { inversionAttempts: 'attemptBoth' });
      if (r && r.binaryData && r.binaryData.length > FOUNTAIN_HEADER) {
        onFrame(Uint8Array.from(r.binaryData));
      }
    }
    setTimeout(tick, 40);
  };
  tick();
  return {
    stop() {
      alive = false;
      for (const t of stream.getTracks()) t.stop();
      video.srcObject = null;
    },
  };
}
