//! Konuşmadaki duygu: emotion2vec+ base (emotion2vec yazarları / FunASR; ONNX, `modeller/emotion2vec/`), işlemcide.
//! 16 kHz ses → 9 sınıf olasılığı. Seslendirmede ZONOS3'ün duygu yönleri olarak yansır.

use std::path::Path;

use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;

/// labels.txt sırası: angry, disgusted, fearful, happy, neutral, other, sad, surprised, <unk>
const ETIKET: [&str; 9] = ["kizgin", "igrenmis", "korkmus", "mutlu", "notr", "diger", "uzgun", "saskin", "notr"];

pub struct Duygu {
    oturum: Session,
}

impl Duygu {
    pub fn ac(model: &Path) -> Result<Self> {
        Ok(Duygu { oturum: crate::onnx::oturum(model, false)?.0 })
    }

    /// 16 kHz cümle sesi → (duygu, olasılık). Çok kısa seste nötr.
    pub fn tahmin(&mut self, x16: &[f32]) -> Result<(String, f32)> {
        if x16.len() < 8000 {
            return Ok(("notr".into(), 1.0));
        }
        let x = &x16[..x16.len().min(16000 * 10)];
        let n = x.len() as f32;
        let ort = x.iter().sum::<f32>() / n;
        let sap = (x.iter().map(|v| (v - ort).powi(2)).sum::<f32>() / n).sqrt().max(1e-5);
        let v: Vec<f32> = x.iter().map(|v| (v - ort) / sap).collect();
        let r = self.oturum.run(ort::inputs!["wav" => Tensor::from_array(([1usize, v.len()], v))?])?;
        let (_, p) = r["probs"].try_extract_tensor::<f32>()?;
        let (mut en, mut deger) = (4usize, 0f32);
        for (i, &q) in p.iter().enumerate().take(9) {
            if q > deger {
                (en, deger) = (i, q);
            }
        }
        Ok((ETIKET[en].to_string(), deger))
    }
}

/// Belirgin duygu mu (seslendirmeye yansıtılacak kadar)?
pub fn belirgin(d: &str, p: f32) -> bool {
    !matches!(d, "notr" | "diger") && p >= 0.55
}
