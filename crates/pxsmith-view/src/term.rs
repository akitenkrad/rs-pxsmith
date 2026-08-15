//! 端末の能力判定と表示 (M1，R1・R18)．
//!
//! 高品質表示は Kitty / iTerm2 / Sixel が前提で，**macOS 標準の Terminal.app は
//! いずれも非対応**である．半ブロック fallback では垂直解像度が半分になり，1px を
//! 確認する用途に耐えない．そのため [`detect`] は「使えるか」ではなく
//! **「1 画素を確認する用途に耐えるか」** ([`TerminalKind::is_pixel_accurate`])
//! を答える．

#[cfg(feature = "terminal")]
use image::{DynamicImage, RgbaImage};
#[cfg(feature = "terminal")]
use viuer::Config;

#[cfg(feature = "terminal")]
use crate::Result;

/// 端末の画像表示能力．
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TerminalKind {
    /// Kitty graphics protocol．
    Kitty,
    /// iTerm2 inline images protocol．
    ITerm,
    /// Sixel (ビルド時の feature が要る)．
    Sixel,
    /// 半ブロック文字による近似．**1px の確認には耐えない**．
    HalfBlock,
}

impl TerminalKind {
    /// 1 画素を確かめる用途に耐えるか．
    pub fn is_pixel_accurate(self) -> bool {
        !matches!(self, Self::HalfBlock)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kitty => "Kitty graphics protocol",
            Self::ITerm => "iTerm2 inline images protocol",
            Self::Sixel => "Sixel",
            Self::HalfBlock => "半ブロック文字 (fallback)",
        }
    }

    /// 人が読む診断文．合否とその理由を返す．
    pub fn report(self) -> String {
        if self.is_pixel_accurate() {
            format!("端末: {} — 1 画素単位の確認に使える", self.as_str())
        } else {
            format!(
                "端末: {} — 垂直解像度が半分で色も近似になるため，\
                 1 画素の確認には耐えない．Kitty / iTerm2 / WezTerm / Ghostty のいずれかへ\
                 移ることを勧める",
                self.as_str()
            )
        }
    }
}

/// 現在の端末を判定する (`terminal` feature)．
///
/// **環境変数で分かるものを先に見る**．Kitty の能力判定は端末へ問い合わせの
/// エスケープ列を書き，応答を読み損ねると画面に生の文字列が残る．iTerm2 や
/// Kitty のように環境変数で分かる場合は問い合わせずに済ませる．
#[cfg(feature = "terminal")]
pub fn detect() -> TerminalKind {
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return TerminalKind::Kitty;
    }
    if viuer::is_iterm_supported() {
        return TerminalKind::ITerm;
    }
    if viuer::get_kitty_support() != viuer::KittySupport::None {
        return TerminalKind::Kitty;
    }
    #[cfg(any(feature = "sixel", feature = "icy_sixel"))]
    if viuer::is_sixel_supported() {
        return TerminalKind::Sixel;
    }
    TerminalKind::HalfBlock
}

/// 表示位置の指定．
#[derive(Copy, Clone, Debug, Default)]
pub struct Placement {
    /// 端末の絶対位置に置く (`pxsmith watch` で画面を書き換えるときに使う)．
    pub absolute: bool,
    pub x: u16,
    pub y: i16,
}

/// 画像を端末に表示する (`terminal` feature)．返り値は使った文字セル数 (幅, 高さ)．
#[cfg(feature = "terminal")]
pub fn show(img: &RgbaImage, placement: Placement) -> Result<(u32, u32)> {
    let config = Config {
        transparent: true,
        absolute_offset: placement.absolute,
        x: placement.x,
        y: placement.y,
        restore_cursor: false,
        // 端末の文字セルへ収める縮小は viuer に任せる．拡大は render 側で
        // ニアレストネイバー済みなので，ここで補間が入っても粒は保たれる
        ..Default::default()
    };
    Ok(viuer::print(
        &DynamicImage::ImageRgba8(img.clone()),
        &config,
    )?)
}

/// 画面を消す (`pxsmith watch` の再描画用)．
pub fn clear() {
    print!("\x1b[2J\x1b[H");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_block_is_not_pixel_accurate() {
        assert!(!TerminalKind::HalfBlock.is_pixel_accurate());
        for k in [
            TerminalKind::Kitty,
            TerminalKind::ITerm,
            TerminalKind::Sixel,
        ] {
            assert!(k.is_pixel_accurate(), "{k:?}");
        }
    }

    #[test]
    fn report_explains_why_half_block_fails() {
        let text = TerminalKind::HalfBlock.report();
        assert!(text.contains("垂直解像度"), "{text}");
        assert!(text.contains("iTerm2"), "移行先が示されていない: {text}");
    }

    // `detect` はここでは呼ばない — Kitty の判定が端末へ問い合わせの
    // エスケープ列を書き，テスト出力を汚す．`pxsmith verify terminal` で確かめる．
}
