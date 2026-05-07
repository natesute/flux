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

/// What the inspector wants the preview loop to do this frame, beyond
/// just reading the (possibly mutated) project.
#[derive(Default)]
pub struct InspectorOutput {
    /// True when the project was edited (any param/topology change).
    pub changed: bool,
    /// One-shot UI actions queued during this frame's UI pass.
    pub actions: Vec<UiAction>,
}

/// User-triggered actions the preview loop dispatches outside the egui
/// closure. Kept as data, not callbacks, so the inspector stays
/// purely data-in / data-out.
pub enum UiAction {
    /// Save the current cooked frame as a PNG.
    Screenshot,
    /// Render `seconds` of video to disk as an mp4 (spawns the offline
    /// render path as a child process so the preview window stays live).
    StartRecord { seconds: f32 },
    /// Save a copy of the current project to a timestamped file in the
    /// user's flux snapshots folder. The active project keeps pointing
    /// at the original — snapshots are forgettable side files.
    Snapshot,
    /// Open a system file dialog and write the current project to the
    /// chosen path. The active project switches to that path so future
    /// edits go there.
    SaveAs,
}

/// State the inspector needs to know to draw correctly but doesn't own.
pub struct InspectorEnv {
    /// Set to `Some(start_time)` while a record-to-mp4 child is in
    /// flight; the inspector greys the record button and shows "rendering…".
    pub recording_since: Option<std::time::Instant>,
    /// Persisted between frames so the duration spinner doesn't reset.
    pub record_seconds: f32,
}

impl Default for InspectorEnv {
    fn default() -> Self {
        Self {
            recording_since: None,
            record_seconds: 10.0,
        }
    }
}

/// Render the inspector and return what changed + what the user clicked.
pub fn ui(ctx: &egui::Context, project: &mut Project, env: &mut InspectorEnv) -> InspectorOutput {
    let mut changed = false;
    let mut actions: Vec<TopologyAction> = Vec::new();
    let mut ui_actions: Vec<UiAction> = Vec::new();

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
            let current_output = project.output.clone();
            ui.horizontal(|ui| {
                ui.label("add");
                egui::ComboBox::from_id_salt("add_node")
                    .selected_text("pick a type…")
                    .show_ui(ui, |ui| {
                        for kind in nodes::registered_names() {
                            if ui.selectable_label(false, kind).clicked() {
                                let name = unique_name(&project.nodes, kind);
                                actions.push(TopologyAction::Add {
                                    inputs: default_inputs_for(
                                        kind,
                                        &existing_names,
                                        &current_output,
                                    ),
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

            // ---- save --------------------------------------------------
            ui.label(egui::RichText::new("save").small().weak());
            ui.horizontal(|ui| {
                if ui
                    .button("✨ snapshot")
                    .on_hover_text(
                        "drop a timestamped copy of the project into \
                         ~/Documents/flux/snapshots/",
                    )
                    .clicked()
                {
                    ui_actions.push(UiAction::Snapshot);
                }
                if ui
                    .button("💾 save as…")
                    .on_hover_text(
                        "save under a new name; the active project switches to that file",
                    )
                    .clicked()
                {
                    ui_actions.push(UiAction::SaveAs);
                }
            });

            ui.add_space(6.0);

            // ---- capture / record --------------------------------------
            ui.label(egui::RichText::new("capture").small().weak());
            ui.horizontal(|ui| {
                if ui
                    .button("📷 screenshot")
                    .on_hover_text("save current frame as PNG to ~/Desktop")
                    .clicked()
                {
                    ui_actions.push(UiAction::Screenshot);
                }
            });
            ui.horizontal(|ui| {
                ui.label("seconds");
                ui.add(
                    egui::DragValue::new(&mut env.record_seconds)
                        .speed(0.5)
                        .range(0.5..=600.0),
                );
                let recording = env.recording_since.is_some();
                let label = if let Some(start) = env.recording_since {
                    format!("● rendering… ({:.0}s)", start.elapsed().as_secs_f32())
                } else {
                    "⏺ render to mp4".to_string()
                };
                let resp = ui.add_enabled(!recording, egui::Button::new(label));
                if resp.clicked() {
                    ui_actions.push(UiAction::StartRecord {
                        seconds: env.record_seconds,
                    });
                }
            });

            ui.add_space(8.0);
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

    InspectorOutput {
        changed,
        actions: ui_actions,
    }
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
            // Pre-populate the new node's params from the central
            // defaults registry (`nodes::default_params_for`). Without
            // this, the spec's params map is empty and the inspector
            // has nothing to render until you've manually typed each
            // key into the .ron.
            let params = nodes::default_params_for(&kind);
            project.nodes.insert(
                name.clone(),
                NodeSpec {
                    kind: kind.clone(),
                    inputs,
                    params,
                },
            );
            // Splice the new node into the visible chain by making it
            // the project's output. Without this the topo sort drops
            // it (it would be unreachable from the previous output)
            // and "add a node" looks like it does nothing. Inputs
            // were smart-defaulted to the previous output upstream,
            // so the chain ends up: …→ old_output → new_node.
            // The user can change the output back via the dropdown.
            tracing::info!("inspector: added node `{name}` (kind={kind}); output → `{name}`");
            project.output = name;
        }
        TopologyAction::Delete { name } => {
            tracing::info!("inspector: deleted node `{name}`");
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

/// Pick reasonable default input wiring for a freshly added node.
///
/// 1-input post effects wire to the *current output* — so adding a
/// `bloom` over an existing chain produces `…→ old_output → new_bloom`,
/// and once the caller flips `project.output` to the new node the
/// chain is intact. 2-input compositors get current_output as the
/// first input and the second-most-recent node as the second.
/// Variable-input nodes (custom_shader) start empty.
fn default_inputs_for(kind: &str, existing: &[String], current_output: &str) -> Vec<String> {
    let count = expected_input_count(kind);
    if count == 0 || existing.is_empty() {
        return Vec::new();
    }
    let cur = current_output.to_string();
    match count {
        1 => vec![cur],
        2 => {
            // Pick a second input that isn't the same as `cur` if we
            // can; otherwise fall back to duplicating it (useful for
            // a self-blend as a starting point).
            let other = existing
                .iter()
                .rev()
                .find(|n| n.as_str() != current_output)
                .cloned()
                .unwrap_or_else(|| cur.clone());
            vec![cur, other]
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
    let defaults = nodes::default_params_for(&kind);

    // Show every param the node *can* take, defaulting to the canonical
    // value when the spec hasn't bound it explicitly. This is what makes
    // sliders appear immediately for newly-added nodes (their spec is
    // empty until the user touches something) and for older .ron files
    // that only listed a few values explicitly. Anything the user
    // touches gets written back to spec.params; untouched defaults stay
    // implicit so the on-disk file doesn't suddenly gain hundreds of
    // explicit baseline values just because someone opened it.
    let mut ordered_keys: Vec<String> = defaults.keys().cloned().collect();
    for k in spec.params.keys() {
        if !ordered_keys.contains(k) {
            ordered_keys.push(k.clone());
        }
    }

    for key in ordered_keys {
        let Some(mut value) = spec
            .params
            .get(&key)
            .cloned()
            .or_else(|| defaults.get(&key).cloned())
        else {
            continue;
        };
        if param_ui(ui, &kind, &key, &mut value) {
            spec.params.insert(key, value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;

    /// Build a one-node project for tests.
    fn one_node_project() -> Project {
        let mut nodes = IndexMap::new();
        nodes.insert(
            "src".to_string(),
            NodeSpec {
                kind: "noise".to_string(),
                inputs: vec![],
                params: nodes::default_params_for("noise"),
            },
        );
        Project {
            version: 1,
            width: 64,
            height: 64,
            fps: 30,
            tone_map: crate::project::ToneMap::Aces,
            nodes,
            output: "src".to_string(),
            source_dir: std::path::PathBuf::from("."),
        }
    }

    /// The bug Nathan hit: adding a node didn't insert it into the
    /// visible chain. apply_action must (a) wire its inputs from the
    /// current output, (b) re-point project.output at the new node,
    /// and (c) populate sensible defaults so its sliders render.
    #[test]
    fn add_inserts_into_chain_and_becomes_output() {
        let mut project = one_node_project();
        let action = TopologyAction::Add {
            name: "post".to_string(),
            kind: "bloom".to_string(),
            inputs: default_inputs_for("bloom", &["src".to_string()], "src"),
        };
        apply_action(&mut project, action);

        // New node exists, with defaults populated.
        let post = project.nodes.get("post").expect("new node added");
        assert_eq!(post.kind, "bloom");
        assert_eq!(post.inputs, vec!["src".to_string()]);
        assert!(
            post.params.contains_key("threshold"),
            "bloom should get its threshold default so the inspector renders a slider"
        );
        assert!(post.params.contains_key("intensity"));
        assert!(post.params.contains_key("radius"));

        // Project output now points at the new node — without this,
        // topo sort drops it and adding does nothing visible.
        assert_eq!(project.output, "post");
    }

    /// Adding a 0-input generator (e.g. a fresh raymarch scene) takes
    /// over as the output with no input wiring needed.
    #[test]
    fn add_generator_becomes_output_with_no_inputs() {
        let mut project = one_node_project();
        let action = TopologyAction::Add {
            name: "scene".to_string(),
            kind: "raymarch".to_string(),
            inputs: default_inputs_for("raymarch", &["src".to_string()], "src"),
        };
        apply_action(&mut project, action);
        let scene = project.nodes.get("scene").unwrap();
        assert_eq!(scene.kind, "raymarch");
        assert!(scene.inputs.is_empty(), "raymarch is a 0-input generator");
        assert_eq!(project.output, "scene");
    }

    /// Deleting a node scrubs references in other nodes' inputs lists.
    #[test]
    fn delete_scrubs_dangling_references() {
        let mut project = one_node_project();
        // Add a 1-input post node wired to `src`.
        apply_action(
            &mut project,
            TopologyAction::Add {
                name: "post".to_string(),
                kind: "bloom".to_string(),
                inputs: vec!["src".to_string()],
            },
        );
        // Delete `src`. The `post` node's inputs should no longer
        // reference it.
        apply_action(
            &mut project,
            TopologyAction::Delete {
                name: "src".to_string(),
            },
        );
        let post = project.nodes.get("post").unwrap();
        assert!(
            !post.inputs.contains(&"src".to_string()),
            "delete should have scrubbed `src` out of post.inputs"
        );
    }
}
