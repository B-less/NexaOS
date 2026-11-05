//! The model. Intent: the features' vectors (DIM numbers each) are
//! averaged and a linear layer gives a score per intent (fastText's
//! design: tiny, fast, good at short commands). Argument: an averaged
//! perceptron tags each word O / B / I, left to right. Stored with the
//! vectors as 8-bit numbers (one scale per vector): about 1 MB.

use crate::data::{Example, INTENTS};
use crate::features::{intent_features, tag_features, words, TAGS};
use crate::rng::Rng;

pub const DIM: usize = 32;
/// Below this the model isn't sure: treat it as not a command (never guess an action).
pub const SURE: f32 = 0.5;
pub const BUCKETS: usize = 1 << 15;
pub const TAG_BUCKETS: usize = 1 << 14;
const MAGIC: &[u8; 4] = b"NXI1";

pub struct Model {
    pub labels: Vec<String>,
    pub emb: Vec<f32>,  // BUCKETS x DIM while training; empty once read from the file
    q: Vec<i8>,         // the same, 8-bit (from the file): 1 MB instead of 4
    qs: Vec<f32>,       // one scale per vector
    pub out: Vec<f32>,  // labels x DIM
    pub bias: Vec<f32>, // labels
    pub tag: Vec<f32>,  // TAG_BUCKETS x TAGS
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// "unknown" when the model isn't sure (p < SURE)
    pub intent: String,
    /// what it would have said, sure or not
    pub raw: String,
    pub p: f32,
    pub second: String,
    pub p2: f32,
    /// The name in the sentence (app, file, workspace), when there is one.
    pub arg: Option<String>,
    /// Words this intent's sentences never use (outside the name): a
    /// request about something else ("lock the front door").
    pub unfamiliar: Vec<String>,
    pub words: Vec<String>,
}

/// Intents whose sentences carry a name for the tagger to find.
pub fn takes_arg(intent: &str) -> bool {
    matches!(intent, "launch" | "find_file" | "workspace_go" | "destroy" | "settings_notifications")
}

fn softmax(v: &mut [f32]) {
    let m = v.iter().cloned().fold(f32::MIN, f32::max);
    let mut s = 0.0;
    for x in v.iter_mut() {
        *x = (*x - m).exp();
        s += *x;
    }
    for x in v.iter_mut() {
        *x /= s;
    }
}

impl Model {
    fn hidden(&self, feats: &[usize]) -> [f32; DIM] {
        let mut h = [0f32; DIM];
        for &f in feats {
            if self.emb.is_empty() {
                let (row, k) = (&self.q[f * DIM..(f + 1) * DIM], self.qs[f]);
                for d in 0..DIM {
                    h[d] += row[d] as f32 * k;
                }
            } else {
                let row = &self.emb[f * DIM..(f + 1) * DIM];
                for d in 0..DIM {
                    h[d] += row[d];
                }
            }
        }
        let n = feats.len().max(1) as f32;
        for x in h.iter_mut() {
            *x /= n;
        }
        h
    }

    fn scores(&self, h: &[f32; DIM]) -> Vec<f32> {
        (0..self.labels.len()).map(|k| {
            let w = &self.out[k * DIM..(k + 1) * DIM];
            self.bias[k] + w.iter().zip(h).map(|(a, b)| a * b).sum::<f32>()
        }).collect()
    }

    fn tag_score(&self, feats: &[usize], t: usize) -> f32 {
        feats.iter().map(|&f| self.tag[f * TAGS + t]).sum()
    }

    /// Tags for each word, left to right (I only after B or I).
    pub fn tags(&self, ws: &[String], intent: &str) -> Vec<usize> {
        let mut prev = 0;
        let mut out = Vec::with_capacity(ws.len());
        for i in 0..ws.len() {
            let f = tag_features(ws, i, intent, prev, TAG_BUCKETS);
            let mut best = (0, f32::MIN);
            for t in 0..TAGS {
                if t == 2 && prev == 0 {
                    continue;
                }
                let s = self.tag_score(&f, t);
                if s > best.1 {
                    best = (t, s);
                }
            }
            out.push(best.0);
            prev = best.0;
        }
        out
    }

    pub fn predict(&self, text: &str) -> Prediction {
        let ws = words(text);
        let mut s = self.scores(&self.hidden(&intent_features(&ws, BUCKETS)));
        softmax(&mut s);
        let mut order: Vec<usize> = (0..s.len()).collect();
        order.sort_by(|a, b| s[*b].partial_cmp(&s[*a]).unwrap_or(std::cmp::Ordering::Equal));
        let (k1, k2) = (order[0], order.get(1).copied().unwrap_or(order[0]));
        let raw = self.labels[k1].clone();
        let intent = if s[k1] < SURE { "unknown".to_string() } else { raw.clone() };
        let arg = if takes_arg(&intent) {
            let t = self.tags(&ws, &intent);
            let start = t.iter().position(|x| *x == 1);
            start.map(|b| {
                let mut e = b + 1;
                while e < t.len() && t[e] == 2 {
                    e += 1;
                }
                ws[b..e].join(" ")
            })
        } else {
            None
        };
        let mut p = Prediction { intent, raw, p: s[k1], second: self.labels[k2].clone(), p2: s[k2], arg, unfamiliar: vec![], words: ws };
        if p.intent != "unknown" {
            p.unfamiliar = crate::vocab::unfamiliar(&p.intent, &p.words, p.arg.as_deref());
            let n = p.unfamiliar.len();
            if n >= 2 || (n == 1 && p.p < 0.9) {
                p.intent = "unknown".into();
                p.arg = None;
            }
        }
        p
    }

    // ---- training ----

    pub fn train(data: &[Example], epochs: usize, seed: u64) -> Model {
        let k = INTENTS.len();
        let mut rng = Rng::new(seed);
        let mut m = Model {
            labels: INTENTS.iter().map(|(n, _)| n.to_string()).collect(),
            emb: (0..BUCKETS * DIM).map(|_| rng.signed() / DIM as f32).collect(),
            out: vec![0.0; k * DIM],
            bias: vec![0.0; k],
            tag: vec![0.0; TAG_BUCKETS * TAGS],
            q: vec![],
            qs: vec![],
        };
        let feats: Vec<Vec<usize>> = data.iter().map(|e| intent_features(&words(&e.text), BUCKETS)).collect();
        let total = (epochs * data.len()) as f32;
        let mut step = 0f32;
        let mut order: Vec<usize> = (0..data.len()).collect();
        for _ in 0..epochs {
            rng.shuffle(&mut order);
            for &i in &order {
                let lr = 0.4 * (1.0 - step / total).max(0.02);
                step += 1.0;
                let f = &feats[i];
                let h = m.hidden(f);
                let mut p = m.scores(&h);
                softmax(&mut p);
                p[data[i].intent] -= 1.0; // the gradient of the loss on the scores
                let mut dh = [0f32; DIM];
                for c in 0..k {
                    let g = p[c];
                    if g.abs() < 1e-6 {
                        continue;
                    }
                    let w = &mut m.out[c * DIM..(c + 1) * DIM];
                    for d in 0..DIM {
                        dh[d] += g * w[d];
                        w[d] -= lr * g * h[d];
                    }
                    m.bias[c] -= lr * g;
                }
                let n = f.len().max(1) as f32;
                for &b in f {
                    let row = &mut m.emb[b * DIM..(b + 1) * DIM];
                    for d in 0..DIM {
                        row[d] -= lr * dh[d] / n;
                    }
                }
            }
        }
        m.train_tagger(data, epochs.max(4), seed ^ 0x9e37);
        m
    }

    fn train_tagger(&mut self, data: &[Example], epochs: usize, seed: u64) {
        let mut rng = Rng::new(seed);
        let n = TAG_BUCKETS * TAGS;
        let (mut w, mut sum, mut stamp) = (vec![0f32; n], vec![0f32; n], vec![0u64; n]);
        let mut clock = 1u64;
        // Every sentence of these intents, with a name or not: it learns when there is none too.
        let with_args: Vec<&Example> = data.iter().filter(|e| takes_arg(INTENTS[e.intent].0)).collect();
        let mut order: Vec<usize> = (0..with_args.len()).collect();
        for _ in 0..epochs {
            rng.shuffle(&mut order);
            for &i in &order {
                let e = with_args[i];
                let ws = words(&e.text);
                if ws.len() != e.arg.len() {
                    continue;
                }
                let gold: Vec<usize> = e.arg.iter().enumerate()
                    .map(|(j, a)| if !a { 0 } else if j > 0 && e.arg[j - 1] { 2 } else { 1 }).collect();
                let intent = INTENTS[e.intent].0;
                let mut prev = 0;
                for j in 0..ws.len() {
                    let f = tag_features(&ws, j, intent, prev, TAG_BUCKETS);
                    let mut best = (0, f32::MIN);
                    for t in 0..TAGS {
                        if t == 2 && prev == 0 {
                            continue;
                        }
                        let s: f32 = f.iter().map(|&b| w[b * TAGS + t]).sum();
                        if s > best.1 {
                            best = (t, s);
                        }
                    }
                    if best.0 != gold[j] {
                        // averaged perceptron, the lazy way: catch up each weight's sum when it changes
                        for &b in &f {
                            for (t, d) in [(gold[j], 1.0f32), (best.0, -1.0)] {
                                let x = b * TAGS + t;
                                sum[x] += w[x] * (clock - stamp[x]) as f32;
                                stamp[x] = clock;
                                w[x] += d;
                            }
                        }
                    }
                    clock += 1;
                    prev = gold[j];
                }
            }
        }
        for x in 0..n {
            sum[x] += w[x] * (clock - stamp[x]) as f32;
            self.tag[x] = sum[x] / clock as f32;
        }
    }

    // ---- the file ----

    /// Vectors as 8-bit numbers with one scale each; the rest as f32.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        for v in [DIM, BUCKETS, TAG_BUCKETS, self.labels.len()] {
            b.extend((v as u32).to_le_bytes());
        }
        for l in &self.labels {
            b.push(l.len() as u8);
            b.extend(l.as_bytes());
        }
        if self.emb.is_empty() {
            for (row, k) in self.q.chunks(DIM).zip(&self.qs) {
                b.extend(k.to_le_bytes());
                b.extend(row.iter().map(|x| *x as u8));
            }
        }
        for row in self.emb.chunks(DIM) {
            let scale = row.iter().fold(0f32, |a, x| a.max(x.abs())) / 127.0;
            b.extend(scale.to_le_bytes());
            for x in row {
                b.push(if scale > 0.0 { (x / scale).round().clamp(-127.0, 127.0) as i8 as u8 } else { 0 });
            }
        }
        for x in self.out.iter().chain(&self.bias).chain(&self.tag) {
            b.extend(x.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Model> {
        if b.get(..4)? != MAGIC {
            return None;
        }
        let mut at = 4;
        let u32_at = |at: &mut usize| -> Option<usize> {
            let v = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?) as usize;
            *at += 4;
            Some(v)
        };
        let (dim, buckets, tb, k) = (u32_at(&mut at)?, u32_at(&mut at)?, u32_at(&mut at)?, u32_at(&mut at)?);
        if dim != DIM || buckets != BUCKETS || tb != TAG_BUCKETS || k == 0 || k > 256 {
            return None;
        }
        let mut labels = Vec::with_capacity(k);
        for _ in 0..k {
            let n = *b.get(at)? as usize;
            labels.push(String::from_utf8(b.get(at + 1..at + 1 + n)?.to_vec()).ok()?);
            at += 1 + n;
        }
        let (mut q, mut qs) = (Vec::with_capacity(buckets * dim), Vec::with_capacity(buckets));
        for _ in 0..buckets {
            qs.push(f32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
            at += 4;
            q.extend(b.get(at..at + dim)?.iter().map(|x| *x as i8));
            at += dim;
        }
        let f32s = |n: usize, at: &mut usize| -> Option<Vec<f32>> {
            let raw = b.get(*at..*at + n * 4)?;
            *at += n * 4;
            Some(raw.chunks(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
        };
        let out = f32s(k * dim, &mut at)?;
        let bias = f32s(k, &mut at)?;
        let tag = f32s(tb * TAGS, &mut at)?;
        (at == b.len()).then_some(Model { labels, emb: vec![], q, qs, out, bias, tag })
    }
}
