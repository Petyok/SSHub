//! Animated QR frames for payloads that do not fit one code (with hybrid wrapping that is the
//! normal case). Not a rateless fountain: the payload is cut into fixed blocks, every block is
//! one frame, and a few XOR parity frames let a receiver repair a single missing block per
//! group without waiting for the next lap. The sender loops the whole frame list forever; a
//! receiver that misses a frame catches it on the next lap.
//!
//! ```text
//! frame = "QF" || doc_id(4) || total_len u32 BE || block_size u16 BE || groups u16 BE || seq u16 BE || block(block_size)
//! ```
//!
//! `doc_id` is the first 4 bytes of BLAKE3 of the payload (it separates documents on screen and
//! checks reassembly, it is not authentication: the challenge inside is signed). Data frames have
//! `seq < k`; the last block is zero padded. `groups` is `G`, the number of parity groups (0 = none),
//! carried in every frame so a receiver never guesses it. With `G` parity groups, parity frame `g` (seq `k + g`)
//! is the XOR of the blocks whose index is congruent to `g` modulo `G`.

use anyhow::{bail, Result};

use super::crypto::chain_hash;

const MAGIC: &[u8; 2] = b"QF";
pub const FRAME_HEADER: usize = 2 + 4 + 4 + 2 + 2 + 2;
/// Payloads above this are refused (a QR animation of this size is already minutes long).
pub const MAX_PAYLOAD: usize = 256 * 1024;

fn doc_id(payload: &[u8]) -> [u8; 4] {
    chain_hash(payload)[..4].try_into().unwrap()
}

/// Number of parity groups for `k` data blocks.
pub fn parity_groups(k: usize, parity_percent: u32) -> usize {
    if k < 2 || parity_percent == 0 {
        0
    } else {
        (k * parity_percent as usize).div_ceil(100).max(1)
    }
}

/// Frames for one lap: all data frames, then the parity frames. Each frame is at most
/// `frame_bytes` long (the QR byte budget).
pub fn fountain_encode(payload: &[u8], frame_bytes: usize, parity_percent: u32) -> Result<Vec<Vec<u8>>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        bail!("payload must be 1..={MAX_PAYLOAD} bytes, got {}", payload.len());
    }
    if frame_bytes <= FRAME_HEADER || frame_bytes - FRAME_HEADER > u16::MAX as usize {
        bail!("frame size {frame_bytes} leaves no room for data");
    }
    let block = frame_bytes - FRAME_HEADER;
    let k = payload.len().div_ceil(block);
    let groups = parity_groups(k, parity_percent);
    let id = doc_id(payload);
    let mk = |seq: usize, data: &[u8]| {
        let mut f = Vec::with_capacity(FRAME_HEADER + block);
        f.extend_from_slice(MAGIC);
        f.extend_from_slice(&id);
        f.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        f.extend_from_slice(&(block as u16).to_be_bytes());
        f.extend_from_slice(&(groups as u16).to_be_bytes());
        f.extend_from_slice(&(seq as u16).to_be_bytes());
        f.extend_from_slice(data);
        f
    };
    let block_at = |i: usize| {
        let mut b = vec![0u8; block];
        let chunk = &payload[i * block..payload.len().min((i + 1) * block)];
        b[..chunk.len()].copy_from_slice(chunk);
        b
    };
    let mut frames: Vec<Vec<u8>> = (0..k).map(|i| mk(i, &block_at(i))).collect();
    for g in 0..groups {
        let mut x = vec![0u8; block];
        for i in (g..k).step_by(groups) {
            for (a, b) in x.iter_mut().zip(block_at(i)) {
                *a ^= b;
            }
        }
        frames.push(mk(k + g, &x));
    }
    Ok(frames)
}

/// What a pushed frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// A block (or a repaired block) was new.
    Progress,
    /// Nothing new: a block we already had, or a parity frame that repaired nothing yet.
    Redundant,
    /// The frame belongs to another document than the one being collected.
    OtherDocument,
}

#[derive(Default)]
pub struct FountainDecoder {
    head: Option<([u8; 4], usize, usize, usize, usize)>, // doc id, total_len, block size, groups, k
    blocks: Vec<Option<Vec<u8>>>,
    parity: Vec<Option<Vec<u8>>>,
}

impl FountainDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `(blocks held, blocks needed)`, or `None` before the first frame.
    pub fn progress(&self) -> Option<(usize, usize)> {
        self.head.map(|(_, _, _, _, k)| (self.blocks.iter().flatten().count(), k))
    }

    pub fn push(&mut self, frame: &[u8]) -> Result<Push> {
        if frame.len() <= FRAME_HEADER || &frame[..2] != MAGIC {
            bail!("not a fountain frame");
        }
        let id: [u8; 4] = frame[2..6].try_into().unwrap();
        let total = u32::from_be_bytes(frame[6..10].try_into().unwrap()) as usize;
        let block = u16::from_be_bytes(frame[10..12].try_into().unwrap()) as usize;
        let groups = u16::from_be_bytes(frame[12..14].try_into().unwrap()) as usize;
        let seq = u16::from_be_bytes(frame[14..16].try_into().unwrap()) as usize;
        if block == 0 || frame.len() != FRAME_HEADER + block || total == 0 || total > MAX_PAYLOAD {
            bail!("malformed fountain frame");
        }
        let k = total.div_ceil(block);
        if groups > k || seq >= k + groups {
            bail!("fountain frame index out of range");
        }
        match self.head {
            None => {
                self.head = Some((id, total, block, groups, k));
                self.blocks = vec![None; k];
                self.parity = vec![None; groups];
            }
            Some((hid, ht, hb, hg, _)) if (hid, ht, hb, hg) != (id, total, block, groups) => return Ok(Push::OtherDocument),
            Some(_) => {}
        }
        let data = frame[FRAME_HEADER..].to_vec();
        let before = self.blocks.iter().flatten().count();
        if seq < k {
            if self.blocks[seq].is_some() {
                return Ok(Push::Redundant);
            }
            self.blocks[seq] = Some(data);
        } else {
            let g = seq - k;
            if self.parity[g].is_some() {
                return Ok(Push::Redundant);
            }
            self.parity[g] = Some(data);
        }
        self.repair();
        Ok(if self.blocks.iter().flatten().count() > before { Push::Progress } else { Push::Redundant })
    }

    /// Repairs a block wherever a group has exactly one missing block and its parity frame.
    fn repair(&mut self) {
        let Some((_, _, block, groups, k)) = self.head else { return };
        for g in 0..groups {
            let Some(p) = &self.parity[g] else { continue };
            let members: Vec<usize> = (g..k).step_by(groups).collect();
            let missing: Vec<usize> = members.iter().copied().filter(|i| self.blocks[*i].is_none()).collect();
            if missing.len() == 1 {
                let mut x = p.clone();
                for i in members.iter().filter(|i| **i != missing[0]) {
                    for (a, b) in x.iter_mut().zip(self.blocks[*i].as_ref().unwrap()) {
                        *a ^= b;
                    }
                }
                debug_assert_eq!(x.len(), block);
                self.blocks[missing[0]] = Some(x);
            }
        }
    }

    /// The payload once every block is held; an error if the reassembly does not match the document id.
    pub fn finish(&self) -> Result<Option<Vec<u8>>> {
        let Some((id, total, _, _, _)) = self.head else { return Ok(None) };
        if self.blocks.iter().any(Option::is_none) {
            return Ok(None);
        }
        let mut out: Vec<u8> = self.blocks.iter().flatten().flatten().copied().collect();
        out.truncate(total);
        if doc_id(&out) != id {
            bail!("reassembled payload does not match its document id (a frame from another source was mixed in)");
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    fn collect(frames: &[Vec<u8>]) -> Option<Vec<u8>> {
        let mut d = FountainDecoder::new();
        for f in frames {
            d.push(f).unwrap();
        }
        d.finish().unwrap()
    }

    #[test]
    fn round_trip_for_awkward_sizes() {
        for n in [1usize, 2, 235, 236, 237, 1000, 4096, 10_000] {
            for parity in [0u32, 25, 100] {
                let frames = fountain_encode(&payload(n), 250, parity).unwrap();
                assert!(frames.iter().all(|f| f.len() <= 250));
                assert_eq!(collect(&frames).unwrap(), payload(n), "n={n} parity={parity}");
            }
        }
    }

    #[test]
    fn order_and_duplicates_do_not_matter() {
        let frames = fountain_encode(&payload(3000), 250, 25).unwrap();
        let mut d = FountainDecoder::new();
        for f in frames.iter().rev().chain(frames.iter()) {
            d.push(f).unwrap();
        }
        assert_eq!(d.finish().unwrap().unwrap(), payload(3000));
    }

    #[test]
    fn a_missing_block_is_repaired_from_parity_without_a_second_lap() {
        let p = payload(2000);
        let frames = fountain_encode(&p, 250, 25).unwrap();
        let k = p.len().div_ceil(250 - FRAME_HEADER);
        // drop one data block per parity group, keep every parity frame
        let groups = parity_groups(k, 25);
        let dropped: Vec<usize> = (0..groups.min(k)).collect();
        let kept: Vec<Vec<u8>> = frames.iter().enumerate().filter(|(i, _)| !dropped.contains(i)).map(|(_, f)| f.clone()).collect();
        assert_eq!(collect(&kept).unwrap(), p);
        // dropping two blocks of one group cannot be repaired by that group alone
        if groups < k {
            let two = [0usize, groups];
            let kept: Vec<Vec<u8>> = frames.iter().enumerate().filter(|(i, _)| !two.contains(i)).map(|(_, f)| f.clone()).collect();
            assert!(collect(&kept).is_none());
        }
    }

    #[test]
    fn an_incomplete_lap_reports_progress_not_a_payload() {
        let frames = fountain_encode(&payload(2000), 250, 0).unwrap();
        let mut d = FountainDecoder::new();
        for f in &frames[..frames.len() - 1] {
            assert_eq!(d.push(f).unwrap(), Push::Progress);
        }
        assert_eq!(d.progress(), Some((frames.len() - 1, frames.len())));
        assert!(d.finish().unwrap().is_none());
        assert_eq!(d.push(&frames[0]).unwrap(), Push::Redundant);
    }

    #[test]
    fn frames_of_another_document_are_ignored_and_corruption_is_caught() {
        let a = fountain_encode(&payload(600), 250, 0).unwrap();
        let b = fountain_encode(&payload(601), 250, 0).unwrap();
        let mut d = FountainDecoder::new();
        d.push(&a[0]).unwrap();
        assert_eq!(d.push(&b[1]).unwrap(), Push::OtherDocument);
        // a frame with the right header but altered content is caught at reassembly
        let mut bad = a[1].clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        d.push(&bad).unwrap();
        for f in &a[2..] {
            d.push(f).unwrap();
        }
        assert!(d.finish().is_err());
    }

    #[test]
    fn malformed_frames_and_bounds() {
        assert!(FountainDecoder::new().push(b"xx").is_err());
        assert!(FountainDecoder::new().push(&[b'Q', b'F', 0, 0, 0, 0, 0, 0, 0, 5, 0, 3, 0, 0, 0, 0, 1, 2]).is_err(), "length does not match block size");
        assert!(fountain_encode(&[], 250, 25).is_err());
        assert!(fountain_encode(&payload(10), FRAME_HEADER, 25).is_err());
        assert!(fountain_encode(&vec![0; MAX_PAYLOAD + 1], 250, 25).is_err());
    }

    #[test]
    fn frame_count_matches_the_budget_arithmetic() {
        // 2.4 KiB challenge at 250 byte frames: 234 data bytes per frame
        let frames = fountain_encode(&payload(2400), 250, 25).unwrap();
        let k = 2400usize.div_ceil(250 - FRAME_HEADER);
        assert_eq!(frames.len(), k + parity_groups(k, 25));
    }
}
