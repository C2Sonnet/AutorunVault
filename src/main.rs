#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Cursor, Write};
use std::os::windows::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

#[derive(Clone)]
struct Drive {
    letter: char,
    name: String,
    fs_type: String,
    drive_type: String,
    capacity: Option<u64>,
    free: Option<u64>,
    ico: Option<PathBuf>,
    inf: Option<PathBuf>,
    inf_preview: String,
    warning: String,
    ico_attrs: Option<u32>,
    inf_attrs: Option<u32>,
    selected: bool,
}

impl Drive {
    fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.letter))
    }
    fn folder_name(&self) -> String {
        format!("{}_{}", self.letter, safe_name(&self.name))
    }
    fn files(&self) -> impl Iterator<Item = (&'static str, &Path)> {
        [("Autorun.ico", self.ico.as_deref()), ("Autorun.inf", self.inf.as_deref())]
            .into_iter()
            .filter_map(|(name, path)| path.map(|p| (name, p)))
    }
    fn detected(&self) -> usize {
        self.ico.is_some() as usize + self.inf.is_some() as usize
    }
}

fn safe_name(s: &str) -> String {
    let name: String = s.chars().map(|c| {
        if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }
    }).collect();
    let clean = name.trim().trim_end_matches(['.', ' ']);
    let clean = if clean.is_empty() { "名称なし" } else { clean };
    // Windows reserved device names cannot be used as a directory component.
    let stem = clean.split('.').next().unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit() && upper.as_bytes()[3] != b'0');
    if reserved { format!("_{clean}") } else { clean.to_owned() }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn from_wide(v: &[u16]) -> String {
    let n = v.iter().position(|&x| x == 0).unwrap_or(v.len());
    String::from_utf16_lossy(&v[..n])
}
fn drive_type_label(kind: u32) -> &'static str {
    match kind {
        2 => "リムーバブル", 3 => "ローカル", 4 => "ネットワーク", 5 => "光学ドライブ",
        6 => "RAMディスク", 1 => "ルートなし", _ => "不明",
    }
}
fn exists_file(p: &Path) -> Option<PathBuf> {
    match fs::metadata(p) { Ok(meta) if meta.is_file() => Some(p.to_owned()), _ => None }
}

fn read_inf(p: &Path) -> String {
    match fs::read(p) {
        Ok(bytes) if bytes.len() > 256 * 1024 => "INFファイルが256KiBを超えたため、プレビューを省略しました。".into(),
        Ok(bytes) => {
            if bytes.starts_with(&[0xFF, 0xFE]) {
                let codes: Vec<u16> = bytes[2..].chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&codes)
            } else if bytes.starts_with(&[0xFE, 0xFF]) {
                let codes: Vec<u16> = bytes[2..].chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&codes)
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            }
        }
        Err(e) => format!("読み取りできません: {e}"),
    }
}

fn scan_drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26u32 {
        if mask & (1u32 << i) == 0 { continue; }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{}:\\", letter);
        let wroot = wide(&root);
        let mut volume = [0u16; 261];
        let mut fs_name = [0u16; 64];
        let mut total = 0u64;
        let mut free = 0u64;
        let kind = unsafe { GetDriveTypeW(wroot.as_ptr()) };
        let volume_ok = unsafe {
            GetVolumeInformationW(
                wroot.as_ptr(), volume.as_mut_ptr(), volume.len() as u32,
                std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(),
                fs_name.as_mut_ptr(), fs_name.len() as u32,
            ) != 0
        };
        let size_ok = unsafe {
            GetDiskFreeSpaceExW(
                wroot.as_ptr(), std::ptr::null_mut(), &mut total, &mut free,
            ) != 0
        };
        let name = if volume_ok { from_wide(&volume) } else { String::new() };
        let name = if name.is_empty() { "名称なし".to_owned() } else { name };
        let dir = PathBuf::from(&root);
        let ico = exists_file(&dir.join("Autorun.ico"));
        let inf = exists_file(&dir.join("Autorun.inf"));
        let inf_preview = inf.as_deref().map(read_inf).unwrap_or_default();
        let warning = if !volume_ok { "ボリューム情報を取得できません（未挿入、アクセス制限など）。".into() }
            else { String::new() };
        drives.push(Drive {
            letter, name, fs_type: if volume_ok { from_wide(&fs_name) } else { "-".into() },
            drive_type: drive_type_label(kind).into(),
            capacity: size_ok.then_some(total), free: size_ok.then_some(free),
            ico_attrs: ico.as_deref().and_then(|p| fs::metadata(p).ok().map(|m| m.file_attributes())),
            inf_attrs: inf.as_deref().and_then(|p| fs::metadata(p).ok().map(|m| m.file_attributes())),
            ico, inf, inf_preview, warning, selected: true,
        });
    }
    drives
}

fn readable_size(v: u64) -> String {
    format!("{:.1} GiB", v as f64 / 1_073_741_824.0)
}
fn destination() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("実行ファイルの保存場所が不明です")?;
    Ok(parent.join("Autorun_Collection"))
}

fn conflicts(drives: &[Drive], dest: &Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    for drive in drives.iter().filter(|d| d.selected) {
        let folder = dest.join(drive.folder_name());
        for (name, _) in drive.files() {
            let target = folder.join(name);
            if target.exists() { paths.push(target); }
        }
    }
    paths
}

// Two-phase save: write a normal-attribute staging file, then replace only after
// user approval. The source's hidden/read-only/system flags remain untouched.
static TEMP_ID: AtomicU64 = AtomicU64::new(1);
fn unique_sidecar(target: &Path, purpose: &str) -> PathBuf {
    let n = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let base = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{base}.kiseki-{purpose}-{pid}-{n}"))
}
fn save_bytes(target: &Path, bytes: &[u8], overwrite: bool) -> Result<&'static str, String> {
    if target.exists() && !overwrite { return Ok("SKIP"); }
    let staged = unique_sidecar(target, "stage");
    let outcome = (|| -> Result<&'static str, String> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&staged)
            .map_err(|e| format!("一時ファイルを作成できません: {e}"))?;
        f.write_all(bytes).map_err(|e| format!("一時ファイルの書込失敗: {e}"))?;
        f.flush().map_err(|e| format!("フラッシュ失敗: {e}"))?;
        drop(f);
        if target.exists() {
            if !overwrite { return Ok("SKIP"); }
            let backup = unique_sidecar(target, "backup");
            fs::rename(target, &backup).map_err(|e| format!("既存ファイルを退避できません: {e}"))?;
            match fs::rename(&staged, target) {
                Ok(()) => {
                    // Backup cleanup may fail if permissions are restrictive.
                    // This does not invalidate the successful replacement.
                    if fs::remove_file(&backup).is_err() {
                        // Backup is owned by this tool, never the original source.
                        if let Ok(meta) = fs::metadata(&backup) {
                            let mut perms = meta.permissions();
                            perms.set_readonly(false);
                            let _ = fs::set_permissions(&backup, perms);
                        }
                        let _ = fs::remove_file(&backup);
                    }
                    Ok("REPLACED")
                }
                Err(e) => {
                    let restore = fs::rename(&backup, target);
                    Err(format!("置換できません: {e}; 復元結果: {restore:?}; 退避先: {}", backup.display()))
                }
            }
        } else {
            fs::rename(&staged, target).map_err(|e| format!("保存できません: {e}"))?;
            Ok("COPIED")
        }
    })();
    let _ = fs::remove_file(&staged);
    outcome
}

fn export(drives: &[Drive], dest: &Path, overwrite: bool) -> String {
    if let Err(e) = fs::create_dir_all(dest) {
        return format!("保存先を作成できません: {} ({e})", dest.display());
    }
    let (mut ok, mut skipped, mut replaced) = (0usize, 0usize, 0usize);
    let mut errors: Vec<String> = vec![];
    let mut report = String::from("Kiseki Autorun Vault v1.2.0 - Export Report\n");
    if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
        report.push_str(&format!("Unix timestamp: {}\n", now.as_secs()));
    }
    report.push_str(&format!("Destination: {}\n\n", dest.display()));
    for drive in drives.iter().filter(|d| d.selected) {
        let folder = dest.join(drive.folder_name());
        report.push_str(&format!("[{}] {} ({})\n", drive.letter, drive.name, drive.drive_type));
        let files: Vec<_> = drive.files().collect();
        if files.is_empty() { report.push_str("  No Autorun files found\n"); continue; }
        if let Err(e) = fs::create_dir_all(&folder) {
            errors.push(format!("{}: フォルダ作成エラー: {e}", folder.display()));
            continue;
        }
        for (name, src) in files {
            let dst = folder.join(name);
            if dst.exists() && !overwrite {
                skipped += 1;
                report.push_str(&format!("  SKIP (exists): {name}\n"));
                continue;
            }
            let result = fs::read(src).map_err(|e| format!("コピー元を読めません: {e}"))
                .and_then(|bytes| save_bytes(&dst, &bytes, overwrite));
            match result {
                Ok("SKIP") => { skipped += 1; report.push_str(&format!("  SKIP: {name}\n")); }
                Ok("REPLACED") => { replaced += 1; ok += 1; report.push_str(&format!("  REPLACED: {name}\n")); }
                Ok(_) => { ok += 1; report.push_str(&format!("  OK: {name}\n")); }
                Err(e) => {
                    report.push_str(&format!("  ERROR: {name}: {e}\n"));
                    errors.push(format!("{} → {}: {e}", src.display(), dst.display()));
                }
            }
        }
    }
    report.push_str(&format!("\nSaved: {ok} (replaced: {replaced})\nSkipped: {skipped}\nErrors: {}\n", errors.len()));
    if !errors.is_empty() {
        report.push_str("\nError details:\n");
        for e in &errors { report.push_str(&format!("- {e}\n")); }
    }
    let report_path = dest.join("Export_Report.txt");
    if let Err(e) = save_bytes(&report_path, report.as_bytes(), overwrite) {
        errors.push(format!("レポート保存エラー: {e}"));
    }
    let mut summary = format!("完了: 保存 {ok} 件（上書き {replaced}） / スキップ {skipped} 件 / エラー {} 件\n保存先: {}", errors.len(), dest.display());
    for e in errors.iter().take(8) { summary.push_str(&format!("\n{e}")); }
    summary
}

fn preview_ico(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() < 6 || &bytes[0..4] != &[0, 0, 1, 0] {
        return Err("ICOヘッダーが正しくありません".into());
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 || count > 256 || bytes.len() < 6 + count * 16 {
        return Err("ICOディレクトリが不正です".into());
    }
    let mut entries = Vec::new();
    for i in 0..count {
        let p = 6 + i * 16;
        let w = if bytes[p] == 0 { 256 } else { bytes[p] as usize };
        let h = if bytes[p + 1] == 0 { 256 } else { bytes[p + 1] as usize };
        let len = u32::from_le_bytes(bytes[p + 8..p + 12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(bytes[p + 12..p + 16].try_into().unwrap()) as usize;
        if let Some(data) = off.checked_add(len).filter(|&end| end <= bytes.len()) {
            if w <= 1024 && h <= 1024 { entries.push((w * h, off, data)); }
        }
    }
    entries.sort_by(|a,b| b.0.cmp(&a.0));
    for (_, off, end) in entries {
        let raw = &bytes[off..end];
        if raw.starts_with(b"\x89PNG\r\n\x1a\n") {
            if let Ok(img) = decode_png(raw) { return Ok(img); }
        }
    }
    // BMP-based ICO images fall back to the image crate.
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
        .map_err(|e| format!("PNG / BMP ICOを表示できません: {e}"))?;
    let rgba = img.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn decode_png(raw: &[u8]) -> Result<egui::ColorImage, String> {
    let mut decoder = png::Decoder::new(Cursor::new(raw));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    if info.width > 1024 || info.height > 1024 { return Err("PNGの画像サイズが大きすぎます".into()); }
    let pixel_count = info.width as usize * info.height as usize;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0],p[1],p[2],255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0],p[0],p[0],p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&v| [v,v,v,255]).collect(),
        _ => return Err("未対応PNGの色形式です".into()),
    };
    if rgba.len() != pixel_count * 4 { return Err("PNG画素数が一致しません".into()); }
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [info.width as usize, info.height as usize], &rgba))
}

fn file_flags(attr: Option<u32>) -> String {
    match attr {
        None => "未検出".into(),
        Some(a) => {
            let mut flags = Vec::new();
            if a & 0x02 != 0 { flags.push("Hidden"); }
            if a & 0x01 != 0 { flags.push("Read-only"); }
            if a & 0x04 != 0 { flags.push("System"); }
            if flags.is_empty() { "通常ファイル".into() } else { flags.join(" · ") }
        }
    }
}

fn card(fill: egui::Color32) -> egui::Frame {
    egui::Frame::default().fill(fill)
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(219,229,232)))
        .corner_radius(14).inner_margin(12.0)
}

struct App {
    drives: Vec<Drive>,
    active: Option<char>,
    textures: std::collections::HashMap<char, egui::TextureHandle>,
    brand_texture: egui::TextureHandle,
    icon_errors: std::collections::HashMap<char, String>,
    overwrite_popup: bool,
    found_conflicts: Vec<PathBuf>,
    status: String,
    destination: PathBuf,
    filter: String,
    show_only_found: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = egui::Color32::from_rgb(244, 248, 251);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(183, 220, 233);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(219, 237, 247);
        style.visuals.selection.bg_fill = egui::Color32::from_rgb(174, 215, 230);
        style.visuals.selection.stroke.color = egui::Color32::from_rgb(38, 82, 108);
        style.spacing.item_spacing = egui::vec2(9.0, 7.0);
        cc.egui_ctx.set_style(style);

        // Windows fonts are only loaded from the local machine (not redistributed).
        for font in [
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
        ] {
            if let Ok(bytes) = fs::read(font) {
                let mut defs = egui::FontDefinitions::default();
                defs.font_data.insert("jp".into(), egui::FontData::from_owned(bytes).into());
                defs.families.entry(egui::FontFamily::Proportional)
                    .or_default().insert(0, "jp".into());
                defs.families.entry(egui::FontFamily::Monospace)
                    .or_default().insert(0, "jp".into());
                cc.egui_ctx.set_fonts(defs);
                break;
            }
        }
        let brand_color = image::load_from_memory(include_bytes!("../assets/app_256.png"))
            .expect("embedded PNG icon must be valid")
            .to_rgba8();
        let brand_image = egui::ColorImage::from_rgba_unmultiplied(
            [brand_color.width() as usize, brand_color.height() as usize],
            brand_color.as_raw(),
        );
        let brand_texture = cc.egui_ctx.load_texture(
            "Kiseki Autorun Vault icon", brand_image, egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            drives: vec![], active: None, textures: Default::default(),
            brand_texture, icon_errors: Default::default(), overwrite_popup: false,
            found_conflicts: vec![], status: String::new(),
            destination: destination().unwrap_or_else(|_| PathBuf::from("Autorun_Collection")),
            filter: String::new(), show_only_found: false,
        };
        app.refresh(&cc.egui_ctx);
        app
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        let old_active = self.active;
        self.drives = scan_drives();
        self.active = old_active.filter(|letter| self.drives.iter().any(|d| d.letter == *letter))
            .or_else(|| self.drives.first().map(|d| d.letter));
        self.textures.clear();
        self.icon_errors.clear();
        for d in &self.drives {
            let Some(path) = &d.ico else { continue };
            let result = fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| {
                    if b.len() > 16 * 1024 * 1024 {
                        return Err("ICOファイルが16MiBを超えています".into());
                    }
                    preview_ico(&b)
                });
            match result {
                Ok(color) => {
                    self.textures.insert(d.letter, ctx.load_texture(
                        format!("autorun_{}", d.letter), color, egui::TextureOptions::LINEAR));
                }
                Err(e) => { self.icon_errors.insert(d.letter, e); }
            }
        }
        let found: usize = self.drives.iter().map(Drive::detected).sum();
        self.status = format!("スキャン完了: {} ドライブ / {} Autorunファイルを検出。",
            self.drives.len(), found);
    }

    fn begin_export(&mut self) {
        if !self.drives.iter().any(|d| d.selected && d.detected() > 0) {
            self.status = "収集対象のAutorunファイルを選択してください。".into();
            return;
        }
        self.found_conflicts = conflicts(&self.drives, &self.destination);
        let report = self.destination.join("Export_Report.txt");
        if report.exists() { self.found_conflicts.push(report); }
        if self.found_conflicts.is_empty() {
            self.status = export(&self.drives, &self.destination, false);
        } else {
            self.overwrite_popup = true;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let ink = egui::Color32::from_rgb(32, 62, 83);
        let muted = egui::Color32::from_rgb(102, 125, 144);
        let white = egui::Color32::WHITE;
        let blue = egui::Color32::from_rgb(79, 153, 194);
        let tint = egui::Color32::from_rgb(231, 243, 250);
        let green = egui::Color32::from_rgb(40, 136, 106);

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::default()
                .fill(egui::Color32::from_rgb(222, 237, 248))
                .inner_margin(egui::Margin::symmetric(18, 12)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.brand_texture.id(), egui::vec2(52.0, 52.0)));
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Kiseki Autorun Vault")
                            .size(23.0).strong().color(ink));
                        ui.label(egui::RichText::new("キセキのオートラン書庫  ·  Windows 11  ·  v1.2.0")
                            .size(12.0).color(muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_sized([154.0, 36.0], egui::Button::new("一括エクスポート")
                            .fill(egui::Color32::from_rgb(158, 210, 233))).clicked() {
                            self.begin_export();
                        }
                        if ui.add_sized([120.0, 36.0], egui::Button::new("再スキャン")
                            .fill(white)).clicked() { self.refresh(ctx); }
                    });
                });
            });

        egui::TopBottomPanel::bottom("status")
            .resizable(true).default_height(80.0).min_height(46.0).max_height(180.0)
            .frame(egui::Frame::default()
                .fill(egui::Color32::from_rgb(229, 241, 248))
                .inner_margin(egui::Margin::symmetric(16, 7)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("実行結果").strong().color(ink));
                    ui.label(egui::RichText::new("元のAutorunファイルは変更しません")
                        .size(11.0).color(muted));
                });
                egui::ScrollArea::vertical().id_salt("status-log")
                    .auto_shrink([false, false]).show(ui, |ui| {
                        ui.label(egui::RichText::new(&self.status).size(12.0).color(ink));
                    });
            });

        // RESIZABLE UPPER PARTITION-CARD PANEL
        egui::TopBottomPanel::top("drive_card_panel")
            .resizable(true).default_height(350.0).min_height(200.0).max_height(580.0)
            .frame(egui::Frame::default()
                .fill(egui::Color32::from_rgb(244, 248, 251))
                .inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ctx, |ui| {
                let detected: usize = self.drives.iter().map(Drive::detected).sum();
                let selected = self.drives.iter().filter(|d| d.selected).count();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("パーティション一覧")
                        .size(17.0).strong().color(ink));
                    ui.label(egui::RichText::new(format!("{}ドライブ  ·  {}ファイル  ·  {}選択中",
                        self.drives.len(), detected, selected)).color(muted).size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_only_found, "検出ありのみ");
                    });
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("ドライブ文字・ボリューム名を検索")
                        .desired_width(310.0));
                    if ui.small_button("すべて選択").clicked() {
                        for d in &mut self.drives { d.selected = true; }
                    }
                    if ui.small_button("検出分を選択").clicked() {
                        for d in &mut self.drives { d.selected = d.detected() > 0; }
                    }
                    if ui.small_button("選択解除").clicked() {
                        for d in &mut self.drives { d.selected = false; }
                    }
                });
                ui.add_space(5.0);
                let needle = self.filter.trim().to_lowercase();
                egui::ScrollArea::vertical().id_salt("partition-cards")
                    .auto_shrink([false, false]).show(ui, |ui| {
                        let gap = 9.0_f32;
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        let available = ui.available_width().max(240.0);
                        let columns = (((available + gap) / 264.0).floor() as usize).clamp(1, 6);
                        let card_width = ((available - gap * (columns - 1) as f32)
                            / columns as f32).max(190.0);
                        ui.horizontal_wrapped(|ui| {
                            for d in &mut self.drives {
                                if self.show_only_found && d.detected() == 0 { continue; }
                                if !needle.is_empty() && !format!("{}: {}", d.letter, d.name)
                                    .to_lowercase().contains(&needle) { continue; }
                                let active = self.active == Some(d.letter);
                                let fill = if active { tint } else { white };
                                let stroke_color = if active { blue }
                                    else { egui::Color32::from_rgb(212, 225, 234) };
                                egui::Frame::default().fill(fill)
                                    .stroke(egui::Stroke::new(if active { 1.8_f32 } else { 1.0_f32 }, stroke_color))
                                    .corner_radius(13).inner_margin(11.0)
                                    .show(ui, |ui| {
                                        ui.set_width(card_width - 24.0);
                                        ui.set_min_height(127.0);
                                        ui.horizontal(|ui| {
                                            ui.checkbox(&mut d.selected, "");
                                            if ui.selectable_label(active,
                                                egui::RichText::new(format!("{}: {}", d.letter, d.name))
                                                    .strong().size(14.0).color(ink)).clicked() {
                                                self.active = Some(d.letter);
                                            }
                                        });
                                        ui.label(egui::RichText::new(format!("{} · {}",
                                            d.drive_type, d.fs_type)).size(11.0).color(muted));
                                        if let (Some(total), Some(free)) = (d.capacity, d.free) {
                                            let usage = if total == 0 { 0.0_f32 } else {
                                                (1.0_f32 - free as f32 / total as f32)
                                                    .clamp(0.0_f32, 1.0_f32)
                                            };
                                            ui.add(egui::ProgressBar::new(usage)
                                                .desired_width(ui.available_width())
                                                .fill(blue));
                                            ui.label(egui::RichText::new(format!("{} / {} 使用",
                                                readable_size(total.saturating_sub(free)),
                                                readable_size(total))).size(10.0).color(muted));
                                        } else {
                                            ui.label(egui::RichText::new("容量情報なし").size(11.0).color(muted));
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(if d.ico.is_some() { "ICO ✓" } else { "ICO —" })
                                                .size(11.0).color(if d.ico.is_some() { green } else { muted }));
                                            ui.label(egui::RichText::new(if d.inf.is_some() { "INF ✓" } else { "INF —" })
                                                .size(11.0).color(if d.inf.is_some() { green } else { muted }));
                                        });
                                    });
                            }
                        });
                    });
            });

        // LOWER DETAILS PANEL: selected drive icon + volume info + INF preview.
        egui::CentralPanel::default()
            .frame(egui::Frame::default()
                .fill(egui::Color32::from_rgb(244, 248, 251))
                .inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("詳細情報プレビュー")
                        .size(18.0).strong().color(ink));
                    ui.label(egui::RichText::new("上のカードのドライブ名をクリックして表示を切り替え")
                        .size(11.0).color(muted));
                });
                ui.add_space(3.0);
                let active = self.drives.iter().find(|d| Some(d.letter) == self.active);
                egui::ScrollArea::vertical().id_salt("details-scroll")
                    .auto_shrink([false, false]).show(ui, |ui| {
                        if let Some(d) = active {
                            ui.columns(2, |cols| {
                                let (left_columns, right_columns) = cols.split_at_mut(1);
                                let left = &mut left_columns[0];
                                let right = &mut right_columns[0];
                                card(white).show(left, |ui| {
                                    ui.horizontal(|ui| {
                                        egui::Frame::default().fill(tint).corner_radius(13)
                                            .inner_margin(8.0).show(ui, |ui| {
                                                if let Some(tex) = self.textures.get(&d.letter) {
                                                    ui.image((tex.id(), egui::vec2(112.0, 112.0)));
                                                } else {
                                                    ui.allocate_ui(egui::vec2(112.0, 112.0), |ui| {
                                                        ui.centered_and_justified(|ui| {
                                                            ui.label(egui::RichText::new("ICOなし")
                                                                .color(muted));
                                                        });
                                                    });
                                                }
                                            });
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new(format!("{}: {}", d.letter, d.name))
                                                .size(19.0).strong().color(ink));
                                            ui.label(format!("{}  ·  {}", d.drive_type, d.fs_type));
                                            ui.monospace(format!("{}", d.root().display()));
                                            if let (Some(total), Some(free)) = (d.capacity, d.free) {
                                                ui.label(format!("容量: {}", readable_size(total)));
                                                ui.label(format!("空き: {}", readable_size(free)));
                                            }
                                        });
                                    });
                                    ui.add_space(6.0);
                                    ui.separator();
                                    for (label, path, attrs) in [
                                        ("Autorun.ico", &d.ico, d.ico_attrs),
                                        ("Autorun.inf", &d.inf, d.inf_attrs),
                                    ] {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(label).strong().color(ink));
                                            ui.colored_label(if path.is_some() { green } else { muted },
                                                if path.is_some() { "検出済み" } else { "未検出" });
                                        });
                                        ui.label(egui::RichText::new(format!("属性: {}", file_flags(attrs)))
                                            .size(11.0).color(muted));
                                        if let Some(p) = path {
                                            if let Ok(meta) = fs::metadata(p) {
                                                ui.label(egui::RichText::new(format!("{} bytes · {}",
                                                    meta.len(), p.display())).size(11.0).color(muted));
                                            }
                                        }
                                        ui.add_space(5.0);
                                    }
                                    if !d.warning.is_empty() { ui.colored_label(egui::Color32::DARK_RED, &d.warning); }
                                    if let Some(err) = self.icon_errors.get(&d.letter) {
                                        ui.colored_label(egui::Color32::DARK_RED, format!("ICOプレビュー不可: {err}"));
                                    }
                                });
                                card(white).show(right, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Autorun.inf · 内容プレビュー")
                                            .size(14.0).strong().color(ink));
                                        if ui.small_button("内容をコピー").clicked() {
                                            ui.ctx().copy_text(d.inf_preview.clone());
                                        }
                                    });
                                    ui.separator();
                                    if d.inf.is_none() {
                                        ui.colored_label(muted, "Autorun.inf はありません。");
                                    } else {
                                        egui::ScrollArea::vertical().id_salt("inf-preview")
                                            .max_height(270.0).show(ui, |ui| {
                                                ui.add(egui::Label::new(egui::RichText::new(&d.inf_preview)
                                                    .monospace().size(12.0).color(ink)).wrap());
                                            });
                                    }
                                });
                            });
                        } else {
                            card(white).show(ui, |ui| {
                                ui.label("ドライブが見つかりません。再スキャンしてください。");
                            });
                        }
                        ui.add_space(9.0);
                        ui.label(egui::RichText::new("保存先").size(13.0).strong().color(ink));
                        ui.label(egui::RichText::new(self.destination.display().to_string())
                            .monospace().size(11.0).color(muted));
                    });
            });

        if self.overwrite_popup {
            let mut keep_open = true;
            egui::Window::new("既存ファイルの上書き確認")
                .collapsible(false).resizable(true).default_width(580.0)
                .open(&mut keep_open).show(ctx, |ui| {
                    ui.colored_label(egui::Color32::from_rgb(170, 88, 56),
                        format!("同名の既存ファイル: {} 件", self.found_conflicts.len()));
                    ui.label("元ドライブのファイルや属性は変更しません。保存先のみ置換します。");
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for p in &self.found_conflicts { ui.small(p.display().to_string()); }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("上書きして保存").clicked() {
                            self.status = export(&self.drives, &self.destination, true);
                            self.overwrite_popup = false;
                        }
                        if ui.button("既存分をスキップ").clicked() {
                            self.status = export(&self.drives, &self.destination, false);
                            self.overwrite_popup = false;
                        }
                        if ui.button("キャンセル").clicked() { self.overwrite_popup = false; }
                    });
                });
            if !keep_open { self.overwrite_popup = false; }
        }
    }
}

fn main() -> eframe::Result<()> {
    // Also set taskbar / Alt+Tab window icon while the executable icon is embedded by build.rs.
    let logo = image::load_from_memory(include_bytes!("../assets/app_256.png"))
        .expect("embedded application icon must be valid").to_rgba8();
    let (width, height) = logo.dimensions();
    let icon = egui::IconData { rgba: logo.into_raw(), width, height };
    eframe::run_native(
        "Kiseki Autorun Vault v1.2.0",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1380.0, 920.0])
                .with_min_inner_size([1020.0, 710.0])
                .with_icon(icon),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
