//! Egui inspector for the live preview. Walks the loaded `Project` and
//! renders a control for every parameter, mutating the project in place.
//! Returns `true` whenever something changed so the preview loop can
//! debounce a save back to disk.
//!
//! The inspector has no model of its own. The on-disk `.ron` is the
//! single source of truth: humans, this UI, and AI agents all edit the
//! same file.

use crate::project::{AudioFeature, NodeSpec, ParamValue, Project, ToneMap};

/// Render the inspector and return whether anything was edited.
pub fn ui(ctx: &egui::Context, project: &mut Project) -> bool {
    let mut changed = false;

    egui::SidePanel::left("inspector")
        .exact_width(320.0)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("flux");
            ui.label(format!(
                "{}×{} @ {} fps",
                project.width, project.height, project.fps
            ));
            ui.separator();

            // Tone map selector — visible right at the top because it
            // changes everything.
            ui.horizontal(|ui| {
                ui.label("tone map");
                let before = project.tone_map;
                egui::ComboBox::from_id_salt("tone_map")
                    .selected_text(tone_map_label(before))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut project.tone_map, ToneMap::Aces, "ACES");
                        ui.selectable_value(&mut project.tone_map, ToneMap::Reinhard, "Reinhard");
                        ui.selectable_value(&mut project.tone_map, ToneMap::None, "none");
                    });
                if before != project.tone_map {
                    changed = true;
                }
            });

            ui.add_space(8.0);
            ui.separator();

            // One collapsing block per node, in declaration order.
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, spec) in project.nodes.iter_mut() {
                    let header = format!("{}  [{}]", name, spec.kind);
                    egui::CollapsingHeader::new(header)
                        .id_salt(name)
                        .default_open(true)
                        .show(ui, |ui| {
                            if node_ui(ui, spec) {
                                changed = true;
                            }
                        });
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.label(
                egui::RichText::new(
                    "Edits auto-save to the .ron file. Agents editing the same \
                     file will see your changes; you'll see theirs.",
                )
                .small()
                .weak(),
            );
        });

    changed
}

fn tone_map_label(t: ToneMap) -> &'static str {
    match t {
        ToneMap::Aces => "ACES",
        ToneMap::Reinhard => "Reinhard",
        ToneMap::None => "none",
    }
}

fn node_ui(ui: &mut egui::Ui, spec: &mut NodeSpec) -> bool {
    let mut changed = false;
    if !spec.inputs.is_empty() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("inputs").small().weak());
            ui.label(spec.inputs.join(", "));
        });
    }
    let kind = spec.kind.clone();
    for (param_name, value) in spec.params.iter_mut() {
        if param_ui(ui, &kind, param_name, value) {
            changed = true;
        }
    }
    changed
}

fn param_ui(ui: &mut egui::Ui, kind: &str, name: &str, value: &mut ParamValue) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(name);

        match value {
            ParamValue::Number(_) | ParamValue::Audio { .. } => {
                if scalar_ui(ui, name, value) {
                    changed = true;
                }
            }
            ParamValue::Color(rgba) => {
                while rgba.len() < 4 {
                    rgba.push(1.0);
                }
                let mut c = [rgba[0], rgba[1], rgba[2], rgba[3]];
                if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                    rgba[0] = c[0];
                    rgba[1] = c[1];
                    rgba[2] = c[2];
                    rgba[3] = c[3];
                    changed = true;
                }
            }
            ParamValue::String(s) => {
                let opts = string_options(kind, name);
                if !opts.is_empty() {
                    egui::ComboBox::from_id_salt((kind, name))
                        .selected_text(s.as_str())
                        .show_ui(ui, |ui| {
                            for opt in opts {
                                if ui.selectable_label(s.as_str() == *opt, *opt).clicked() {
                                    *s = opt.to_string();
                                    changed = true;
                                }
                            }
                        });
                } else if ui.text_edit_singleline(s).changed() {
                    changed = true;
                }
            }
        }
    });
    changed
}

/// Toggle button + scalar widget. When "audio" is on the param is an
/// `Audio { feature, scale, bias }`; off, it collapses to a `Number`.
/// Switching modes preserves the current effective value (the audio
/// `bias` becomes the number, and vice versa).
fn scalar_ui(ui: &mut egui::Ui, name: &str, value: &mut ParamValue) -> bool {
    let mut changed = false;
    let is_audio = matches!(value, ParamValue::Audio { .. });

    let mut audio_now = is_audio;
    if ui
        .toggle_value(&mut audio_now, "🎵")
        .on_hover_text("audio bind")
        .changed()
    {
        if audio_now {
            // Number → Audio (default to bass, bias = current).
            let cur = match value {
                ParamValue::Number(n) => *n,
                _ => 0.0,
            };
            *value = ParamValue::Audio {
                feature: AudioFeature::Bass,
                scale: 1.0,
                bias: cur,
            };
        } else {
            let cur = match value {
                ParamValue::Audio { bias, .. } => *bias,
                _ => 0.0,
            };
            *value = ParamValue::Number(cur);
        }
        changed = true;
    }

    match value {
        ParamValue::Number(n) => {
            changed |= ui
                .add(egui::DragValue::new(n).speed(speed_for(name)))
                .changed();
        }
        ParamValue::Audio {
            feature,
            scale,
            bias,
        } => {
            egui::ComboBox::from_id_salt(("feat", name))
                .selected_text(feature_label(*feature))
                .width(60.0)
                .show_ui(ui, |ui| {
                    for f in [
                        AudioFeature::Rms,
                        AudioFeature::Bass,
                        AudioFeature::LowMid,
                        AudioFeature::HighMid,
                        AudioFeature::Treble,
                    ] {
                        if ui
                            .selectable_label(*feature == f, feature_label(f))
                            .clicked()
                        {
                            *feature = f;
                            changed = true;
                        }
                    }
                });
            ui.label("×");
            changed |= ui
                .add(egui::DragValue::new(scale).speed(0.01))
                .on_hover_text("scale")
                .changed();
            ui.label("+");
            changed |= ui
                .add(egui::DragValue::new(bias).speed(speed_for(name)))
                .on_hover_text("bias")
                .changed();
        }
        _ => {}
    }
    changed
}

fn feature_label(f: AudioFeature) -> &'static str {
    match f {
        AudioFeature::Rms => "rms",
        AudioFeature::Bass => "bass",
        AudioFeature::LowMid => "low_mid",
        AudioFeature::HighMid => "high_mid",
        AudioFeature::Treble => "treble",
    }
}

/// Sensible drag speed per param name, since one-size-fits-all is annoying
/// (e.g. `radius` at scale 0.01 takes forever; `decay` at speed 0.5 is
/// hopeless to fine-tune).
fn speed_for(name: &str) -> f32 {
    match name {
        // Sub-unit knobs that want 0.001..0.05 changes per pixel.
        "decay" | "zoom" | "rotation" | "amount" | "displacement" | "intensity" | "saturation"
        | "contrast" | "gain" | "brightness" | "factor" | "opacity" | "mix_in" | "scale_x"
        | "scale_y" | "offset_x" | "offset_y" => 0.005,
        // Scene-scale knobs.
        "radius" | "fov" | "scale" | "speed" | "octaves" | "threshold" | "audio_drive"
        | "base_scale" => 0.02,
        // Camera in world units.
        "cam_x" | "cam_y" | "cam_z" | "look_x" | "look_y" | "look_z" | "light_x" | "light_y"
        | "light_z" => 0.05,
        _ => 0.01,
    }
}

fn string_options(kind: &str, name: &str) -> &'static [&'static str] {
    match (kind, name) {
        ("blend", "mode") => &["over", "add", "multiply", "screen", "mix"],
        ("displace", "mode") => &["derivative", "vector"],
        _ => &[],
    }
}
