//! Egui inspector for the live preview. Walks the loaded `Project` and
//! renders controls for every parameter, an "add node" palette, a delete
//! button per node, and per-slot dropdowns for input wiring. All edits
//! mutate the project in place; the preview's render loop debounces a
//! save to disk.
//!
//! The inspector has no model of its own. The on-disk `.ron` is the
//! single source of truth: humans, this UI, and AI agents all edit the
//! same file.
//!
//! Topology edits (add / delete / re-wire) are collected during the UI
//! pass and applied after the egui closure exits — egui already owns a
//! `&mut Project` for the param widgets, so iterating `project.nodes`
//! mutably *and* inserting/removing entries inside that loop would
//! invalidate the iteration. Two-phase keeps it simple.

use indexmap::IndexMap;

use crate::nodes;
use crate::project::{AudioFeature, NodeSpec, ParamValue, Project, ToneMap};

/// Render the inspector and return whether anything was edited.
pub fn ui(ctx: &egui::Context, project: &mut Project) -> bool {
    let mut changed = false;
    let mut actions: Vec<TopologyAction> = Vec::new();

    egui::SidePanel::left("inspector")
        .exact_width(320.0)
        .resizable(false)
        .show(ctx, |ui| {
            // ---- header ------------------------------------------------
            ui.heading("flux");
            ui.label(format!(
                "{}×{} @ {} fps",
                project.width, project.height, project.fps
            ));
            ui.separator();

            // ---- project-level controls --------------------------------
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

            // Output picker — which node's texture is the final frame.
            ui.horizontal(|ui| {
                ui.label("output");
                let before = project.output.clone();
                egui::ComboBox::from_id_salt("output_pick")
                    .selected_text(&project.output)
                    .show_ui(ui, |ui| {
                        for name in project.nodes.keys() {
                            ui.selectable_value(&mut project.output, name.clone(), name);
                        }
                    });
                if before != project.output {
                    changed = true;
                }
            });

            // ---- add-node palette --------------------------------------
            ui.add_space(6.0);
            let existing_names: Vec<String> = project.nodes.keys().cloned().collect();
            ui.horizontal(|ui| {
                ui.label("add");
                egui::ComboBox::from_id_salt("add_node")
                    .selected_text("pick a type…")
                    .show_ui(ui, |ui| {
                        for kind in nodes::registered_names() {
                            if ui.selectable_label(false, kind).clicked() {
                                let name = unique_name(&project.nodes, kind);
                                actions.push(TopologyAction::Add {
                                    inputs: default_inputs_for(kind, &existing_names),
                                    name,
                                    kind: kind.to_string(),
                                });
                            }
                        }
                    });
            });

            ui.add_space(8.0);
            ui.separator();

            // ---- per-node panels ---------------------------------------
            let names: Vec<String> = project.nodes.keys().cloned().collect();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, spec) in project.nodes.iter_mut() {
                    let header = format!("{}  [{}]", name, spec.kind);
                    let is_output = name == &project.output;
                    egui::CollapsingHeader::new(header)
                        .id_salt(name)
                        .default_open(true)
                        .show(ui, |ui| {
                            // Inputs editor.
                            if input_ui(ui, name, spec, &names) {
                                changed = true;
                            }
                            ui.separator();
                            // Params.
                            if node_params_ui(ui, spec) {
                                changed = true;
                            }
                            ui.separator();
                            // Delete row.
                            ui.horizontal(|ui| {
                                let btn = egui::Button::new(egui::RichText::new("delete").small());
                                let resp = ui.add_enabled(!is_output, btn);
                                if is_output {
                                    resp.on_hover_text(
                                        "this is the output — change `output` first",
                                    );
                                } else if resp.clicked() {
                                    actions.push(TopologyAction::Delete { name: name.clone() });
                                }
                            });
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

    // ---- apply queued topology actions -------------------------------
    if !actions.is_empty() {
        for action in actions {
            apply_action(project, action);
        }
        changed = true;
    }

    changed
}

/// Mutations to the `nodes` IndexMap collected during the UI pass and
/// applied after — avoids borrow conflicts with the per-node iteration.
enum TopologyAction {
    Add {
        name: String,
        kind: String,
        inputs: Vec<String>,
    },
    Delete {
        name: String,
    },
}

fn apply_action(project: &mut Project, action: TopologyAction) {
    match action {
        TopologyAction::Add { name, kind, inputs } => {
            project.nodes.insert(
                name,
                NodeSpec {
                    kind,
                    inputs,
                    params: IndexMap::new(),
                },
            );
        }
        TopologyAction::Delete { name } => {
            project.nodes.shift_remove(&name);
            // Scrub stale references so a delete leaves the graph in a
            // valid state instead of a "unknown input node" error.
            for (_, spec) in project.nodes.iter_mut() {
                spec.inputs.retain(|i| i != &name);
            }
        }
    }
}

/// Number of inputs a node of `kind` requires. Used to auto-wire newly
/// added nodes to sensible defaults so adding a `feedback` doesn't
/// silently fail with "requires exactly 1 input."
fn expected_input_count(kind: &str) -> usize {
    match kind {
        "blend" | "displace" => 2,
        "feedback"
        | "bloom"
        | "transform"
        | "levels"
        | "chromatic_aberration"
        | "grain"
        | "color_grade" => 1,
        // 0-input generators (solid/gradient/noise/raymarch/instance) and
        // custom_shader (variable; user wires by hand).
        _ => 0,
    }
}

/// Pick reasonable default input wiring for a freshly added node:
/// most-recent existing node for single-input ops, the two most recent
/// for two-input ops. Returns `vec![]` when there's nothing to wire to,
/// in which case the user has to add inputs by hand once they have
/// other nodes.
fn default_inputs_for(kind: &str, existing: &[String]) -> Vec<String> {
    let count = expected_input_count(kind);
    if count == 0 || existing.is_empty() {
        return Vec::new();
    }
    let last = existing.last().unwrap().clone();
    match count {
        1 => vec![last],
        2 => {
            let prev = if existing.len() >= 2 {
                existing[existing.len() - 2].clone()
            } else {
                last.clone()
            };
            vec![prev, last]
        }
        n => existing.iter().rev().take(n).rev().cloned().collect(),
    }
}

fn unique_name(nodes: &IndexMap<String, NodeSpec>, kind: &str) -> String {
    if !nodes.contains_key(kind) {
        return kind.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{kind}_{n}");
        if !nodes.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{kind}_{}", nodes.len() + 1)
}

fn tone_map_label(t: ToneMap) -> &'static str {
    match t {
        ToneMap::Aces => "ACES",
        ToneMap::Reinhard => "Reinhard",
        ToneMap::None => "none",
    }
}

/// Render the inputs section. Each existing input is a dropdown of all
/// other nodes; +/− buttons add and remove slots. Returns whether
/// anything changed.
fn input_ui(ui: &mut egui::Ui, self_name: &str, spec: &mut NodeSpec, all_names: &[String]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("inputs").small().weak());
        if ui.small_button("+").clicked() {
            // Default new slot to first available non-self node.
            let default = all_names
                .iter()
                .find(|n| n.as_str() != self_name)
                .cloned()
                .unwrap_or_default();
            spec.inputs.push(default);
            changed = true;
        }
    });

    let mut remove: Option<usize> = None;
    for (i, input) in spec.inputs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("  [{i}]"));
            egui::ComboBox::from_id_salt(("input", self_name, i))
                .selected_text(input.as_str())
                .show_ui(ui, |ui| {
                    for n in all_names {
                        if n == self_name {
                            continue;
                        }
                        if ui.selectable_label(input == n, n).clicked() {
                            *input = n.clone();
                            changed = true;
                        }
                    }
                });
            if ui.small_button("−").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        spec.inputs.remove(i);
        changed = true;
    }
    changed
}

fn node_params_ui(ui: &mut egui::Ui, spec: &mut NodeSpec) -> bool {
    let mut changed = false;
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

/// Sensible drag speed per param name.
fn speed_for(name: &str) -> f32 {
    match name {
        "decay" | "zoom" | "rotation" | "amount" | "displacement" | "intensity" | "saturation"
        | "contrast" | "gain" | "brightness" | "factor" | "opacity" | "mix_in" | "scale_x"
        | "scale_y" | "offset_x" | "offset_y" => 0.005,
        "radius" | "fov" | "scale" | "speed" | "octaves" | "threshold" | "audio_drive"
        | "base_scale" => 0.02,
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
