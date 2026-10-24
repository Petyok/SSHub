// Inserted into captureDoc() of assets/companion/scripts/e2e.mjs (branch poc/phone), right after
// `const r = jsQR(px, shot.w, shot.h, ...)`: dumps every distinct frame of the page's #reverse-qr canvas.
      if (r && sel === '#reverse-qr') {
        const key = Buffer.from(r.binaryData.slice(0, 8)).toString('hex');
        globalThis.__seen ??= new Set();
        if (!globalThis.__seen.has(key) && globalThis.__seen.size < 16) {
          globalThis.__seen.add(key);
          const rowBytes = Math.ceil(shot.w / 8);
          const pbm = Buffer.alloc(rowBytes * shot.h);
          for (let y = 0; y < shot.h; y++) for (let x = 0; x < shot.w; x++) if (px[(y * shot.w + x) * 4] < 128) pbm[y * rowBytes + (x >> 3)] |= 0x80 >> (x & 7);
          fs.mkdirSync(path.join(OUT, 'qr'), { recursive: true });
          fs.writeFileSync(path.join(OUT, 'qr', `frame-${String(globalThis.__seen.size - 1).padStart(2, '0')}.pbm`), Buffer.concat([Buffer.from(`P4\n${shot.w} ${shot.h}\n`), pbm]));
        }
      }
