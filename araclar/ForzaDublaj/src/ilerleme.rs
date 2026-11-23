//! Aşamaların arayüze ilerleme bildirmesi ve iptal.
//!   Bildir: (durum, yüzde 0..100, ayrıntı) — durum "indiriliyor" (model yükleniyor / bekleniyor) ya da "calisiyor".
//!   İptal: iş yöneticisi kendi iş parçacığına bir bayrak koyar; uzun döngüler `kontrol()?` ile bakar.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub type Bildir<'a> = &'a mut dyn FnMut(&str, f32, &str);

/// Hiçbir şey bildirmeyen (geliştirme komutları için)
pub fn sessiz() -> impl FnMut(&str, f32, &str) {
    |_: &str, _: f32, _: &str| {}
}

#[derive(Debug)]
pub struct Iptal;

impl std::fmt::Display for Iptal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("iptal edildi")
    }
}

impl std::error::Error for Iptal {}

thread_local! {
    static BAYRAK: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// İş iptal edildiyse Iptal hatası
pub fn kontrol() -> anyhow::Result<()> {
    let iptal = BAYRAK.with(|x| x.borrow().as_ref().is_some_and(|b| b.load(Ordering::Relaxed)));
    if iptal { Err(Iptal.into()) } else { Ok(()) }
}
