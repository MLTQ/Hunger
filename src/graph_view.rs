use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    time::Instant,
};

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use glam::{Quat, Vec3};

use crate::models::{DashboardSnapshot, PageRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GraphColorMode {
    Novelty,
    PageSemantic,
    JudgmentSemantic,
}

impl Default for GraphColorMode {
    fn default() -> Self {
        Self::Novelty
    }
}

impl GraphColorMode {
    pub const ALL: [Self; 3] = [Self::Novelty, Self::PageSemantic, Self::JudgmentSemantic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Novelty => "NOVELTY",
            Self::PageSemantic => "PAGE SEM",
            Self::JudgmentSemantic => "LLM SEM",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Novelty => "hyperlink topology + novelty heat",
            Self::PageSemantic => "content embeddings // semantic clusters",
            Self::JudgmentSemantic => "llm judgment embeddings // response resonance",
        }
    }
}

pub struct GraphViewport {
    yaw: f32,
    pitch: f32,
    distance: f32,
    last_tick: Instant,
    nodes: HashMap<String, NodePhysics>,
    semantic_cache: HashMap<GraphColorMode, CachedSemanticState>,
    telemetry: FieldTelemetry,
}

struct NodePhysics {
    position: Vec3,
    velocity: Vec3,
}

struct CachedSemanticState {
    signature: u64,
    state: SemanticState,
}

#[derive(Clone, Default)]
struct SemanticState {
    clusters: HashMap<String, usize>,
    resonance_pairs: Vec<SemanticPair>,
    coverage: usize,
}

#[derive(Clone, Default)]
pub struct FieldTelemetry {
    pub mode: GraphColorMode,
    pub total_nodes: usize,
    pub coverage: usize,
    pub resonance_links: usize,
    pub average_energy: f32,
    pub max_energy: f32,
    pub cluster_sizes: Vec<ClusterTelemetry>,
    pub top_domains: Vec<DomainTelemetry>,
}

#[derive(Clone, Default)]
pub struct ClusterTelemetry {
    pub cluster_index: usize,
    pub count: usize,
}

#[derive(Clone, Default)]
pub struct DomainTelemetry {
    pub domain: String,
    pub count: usize,
}

#[derive(Clone)]
struct SemanticPair {
    left_url: String,
    right_url: String,
    similarity: f32,
}

impl Default for GraphViewport {
    fn default() -> Self {
        Self {
            yaw: 0.48,
            pitch: 0.34,
            distance: 4.8,
            last_tick: Instant::now(),
            nodes: HashMap::new(),
            semantic_cache: HashMap::new(),
            telemetry: FieldTelemetry::default(),
        }
    }
}

impl GraphViewport {
    pub fn draw(&mut self, ui: &mut egui::Ui, snapshot: &DashboardSnapshot, mode: GraphColorMode) {
        let available = ui.available_size();
        let desired = egui::vec2(available.x.max(480.0), available.y.max(320.0));
        let (rect, response) = ui.allocate_exact_size(desired, Sense::drag());
        self.handle_input(ui, &response);
        self.sync_nodes(snapshot);
        let semantic_state = self.semantic_state(snapshot, mode);
        self.step_layout(snapshot, mode, &semantic_state);
        self.telemetry = build_field_telemetry(snapshot, mode, &semantic_state);
        let hover_pos = response.hover_pos();
        self.paint(ui, rect, snapshot, hover_pos, mode, &semantic_state);
    }

    pub fn telemetry(&self) -> FieldTelemetry {
        self.telemetry.clone()
    }

    fn sync_nodes(&mut self, snapshot: &DashboardSnapshot) {
        let live_urls = snapshot
            .graph_nodes
            .iter()
            .map(|node| node.url.as_str())
            .collect::<HashSet<_>>();
        self.nodes.retain(|url, _| live_urls.contains(url.as_str()));

        for node in &snapshot.graph_nodes {
            self.nodes
                .entry(node.url.clone())
                .or_insert_with(|| NodePhysics {
                    position: seeded_position(&node.url),
                    velocity: Vec3::ZERO,
                });
        }
    }

    fn step_layout(
        &mut self,
        snapshot: &DashboardSnapshot,
        mode: GraphColorMode,
        semantic_state: &SemanticState,
    ) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().clamp(0.008, 0.033);
        self.last_tick = now;

        let urls = snapshot
            .graph_nodes
            .iter()
            .map(|node| node.url.clone())
            .collect::<Vec<_>>();
        let mut positions = urls
            .iter()
            .map(|url| {
                self.nodes
                    .get(url)
                    .map(|node| node.position)
                    .unwrap_or(Vec3::ZERO)
            })
            .collect::<Vec<_>>();
        let mut velocities = urls
            .iter()
            .map(|url| {
                self.nodes
                    .get(url)
                    .map(|node| node.velocity)
                    .unwrap_or(Vec3::ZERO)
            })
            .collect::<Vec<_>>();
        let mut forces = vec![Vec3::ZERO; urls.len()];

        let edge_set = snapshot
            .graph_edges
            .iter()
            .map(|edge| (edge.source_url.as_str(), edge.target_url.as_str()))
            .collect::<HashSet<_>>();
        let url_to_index = urls
            .iter()
            .enumerate()
            .map(|(index, url)| (url.as_str(), index))
            .collect::<HashMap<_, _>>();

        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                let delta = positions[j] - positions[i];
                let distance_sq = delta.length_squared().max(0.018);
                let direction = delta / distance_sq.sqrt();
                let repulsion = 0.09 / distance_sq;
                forces[i] -= direction * repulsion;
                forces[j] += direction * repulsion;
            }
        }

        for i in 0..urls.len() {
            for j in (i + 1)..urls.len() {
                let edge_forward = edge_set.contains(&(urls[i].as_str(), urls[j].as_str()));
                let edge_reverse = edge_set.contains(&(urls[j].as_str(), urls[i].as_str()));
                if !(edge_forward || edge_reverse) {
                    continue;
                }

                let delta = positions[j] - positions[i];
                let distance = delta.length().max(0.01);
                let direction = delta / distance;
                let target = 0.44;
                let attraction = (distance - target) * 0.55;
                forces[i] += direction * attraction;
                forces[j] -= direction * attraction;
            }
        }

        if mode != GraphColorMode::Novelty {
            apply_semantic_forces(semantic_state, &url_to_index, &positions, &mut forces, mode);
        }

        for (index, force) in forces.iter_mut().enumerate() {
            *force += -positions[index] * 0.12;
            force.z += ((index as f32 * 0.618_034).sin()) * 0.01;
        }

        for index in 0..positions.len() {
            velocities[index] = (velocities[index] + forces[index] * dt) * 0.92;
            positions[index] += velocities[index] * dt;
        }

        for (index, url) in urls.iter().enumerate() {
            if let Some(node) = self.nodes.get_mut(url) {
                node.position = positions[index];
                node.velocity = velocities[index];
            }
        }
    }

    fn semantic_state(
        &mut self,
        snapshot: &DashboardSnapshot,
        mode: GraphColorMode,
    ) -> SemanticState {
        if mode == GraphColorMode::Novelty {
            self.semantic_cache
                .entry(mode)
                .or_insert_with(|| CachedSemanticState {
                    signature: 0,
                    state: SemanticState::default(),
                });
            return self
                .semantic_cache
                .get(&mode)
                .expect("novelty cache")
                .state
                .clone();
        }

        let signature = semantic_signature(snapshot, mode);
        let cached = self
            .semantic_cache
            .entry(mode)
            .or_insert_with(|| CachedSemanticState {
                signature,
                state: build_semantic_state(snapshot, mode),
            });

        if cached.signature != signature {
            cached.signature = signature;
            cached.state = build_semantic_state(snapshot, mode);
        }

        cached.state.clone()
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        snapshot: &DashboardSnapshot,
        hover_pos: Option<Pos2>,
        mode: GraphColorMode,
        semantic_state: &SemanticState,
    ) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(5, 8, 15));
        draw_grid(&painter, rect);
        draw_scope_rings(&painter, rect, mode);

        let mut projected = snapshot
            .graph_nodes
            .iter()
            .filter_map(|node| {
                let physics = self.nodes.get(&node.url)?;
                project(
                    rect,
                    physics.position,
                    self.yaw,
                    self.pitch,
                    self.distance,
                    node.food_energy,
                )
                .map(|projection| (node, projection))
            })
            .collect::<Vec<_>>();

        projected.sort_by(|(_, left), (_, right)| left.depth.total_cmp(&right.depth));

        let projection_by_url = projected
            .iter()
            .map(|(node, projection)| (node.url.as_str(), projection.clone()))
            .collect::<HashMap<_, _>>();

        let hovered = hover_pos.and_then(|pointer| {
            projected
                .iter()
                .rev()
                .find(|(_, projection)| {
                    projection.screen.distance(pointer) <= projection.radius + 4.0
                })
                .and_then(|(node, projection)| {
                    snapshot
                        .pages
                        .iter()
                        .find(|page| page.url == node.url)
                        .map(|page| HoveredNode {
                            page,
                            position: pointer + egui::vec2(14.0, 14.0),
                            radius: projection.radius,
                        })
                })
        });

        for edge in &snapshot.graph_edges {
            let Some(source) = projection_by_url.get(edge.source_url.as_str()) else {
                continue;
            };
            let Some(target) = projection_by_url.get(edge.target_url.as_str()) else {
                continue;
            };
            let alpha = ((source.fade + target.fade) * 0.5 * 76.0) as u8;
            painter.line_segment(
                [source.screen, target.screen],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 210, 255, alpha)),
            );
        }

        if mode != GraphColorMode::Novelty {
            draw_semantic_resonance(&painter, &projection_by_url, semantic_state, mode);
        }

        for (node, projection) in projected {
            let cluster = semantic_state.clusters.get(&node.url).copied();
            let fill = node_color(mode, node.food_energy, projection.fade, cluster);
            let stroke = node_stroke(mode, projection.fade, cluster);
            painter.circle_filled(projection.screen, projection.radius, fill);
            painter.circle_stroke(projection.screen, projection.radius + 1.6, stroke);

            if projection.fade > 0.66 {
                painter.text(
                    projection.screen + egui::vec2(0.0, projection.radius + 8.0),
                    egui::Align2::CENTER_TOP,
                    &node.domain,
                    egui::FontId::proportional(10.0),
                    Color32::from_rgba_unmultiplied(223, 235, 247, (projection.fade * 210.0) as u8),
                );
            }
        }

        painter.text(
            rect.left_top() + egui::vec2(18.0, 14.0),
            egui::Align2::LEFT_TOP,
            "NETWORK FIELD",
            egui::FontId::proportional(18.0),
            Color32::from_rgb(227, 234, 242),
        );
        painter.text(
            rect.left_top() + egui::vec2(18.0, 38.0),
            egui::Align2::LEFT_TOP,
            mode.subtitle(),
            egui::FontId::monospace(11.0),
            Color32::from_rgb(111, 138, 160),
        );
        painter.text(
            rect.left_top() + egui::vec2(18.0, 56.0),
            egui::Align2::LEFT_TOP,
            "drag to orbit  |  wheel to zoom",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(111, 138, 160),
        );

        if mode != GraphColorMode::Novelty {
            painter.text(
                rect.right_top() + egui::vec2(-18.0, 18.0),
                egui::Align2::RIGHT_TOP,
                format!(
                    "coverage {:02}/{}",
                    semantic_state.coverage,
                    snapshot.graph_nodes.len()
                ),
                egui::FontId::monospace(11.0),
                Color32::from_rgb(255, 193, 106),
            );
        }

        if let Some(hovered) = hovered {
            draw_tooltip(ui.ctx(), hovered, mode, semantic_state);
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged() {
            let delta = ui.input(|input| input.pointer.delta());
            self.yaw += delta.x * 0.008;
            self.pitch = (self.pitch - delta.y * 0.008).clamp(-1.25, 1.25);
        }

        if response.hovered() {
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            if scroll.abs() > f32::EPSILON {
                self.distance = (self.distance - scroll * 0.004).clamp(2.2, 9.5);
            }
        }
    }
}

#[derive(Clone)]
struct Projection {
    screen: Pos2,
    depth: f32,
    radius: f32,
    fade: f32,
}

struct HoveredNode<'a> {
    page: &'a PageRecord,
    position: Pos2,
    radius: f32,
}

fn project(
    rect: Rect,
    position: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    energy: f32,
) -> Option<Projection> {
    let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let rotated = rotation * position;
    let camera_depth = rotated.z + distance;
    if camera_depth <= 0.12 {
        return None;
    }

    let scale = rect.width().min(rect.height()) * 0.42;
    let perspective = scale / camera_depth;
    let screen = Pos2::new(
        rect.center().x + rotated.x * perspective,
        rect.center().y - rotated.y * perspective,
    );
    let fade = (1.35 - camera_depth / (distance + 1.0)).clamp(0.18, 1.0);
    let radius = (4.0 + energy * 11.0) * fade;

    Some(Projection {
        screen,
        depth: camera_depth,
        radius,
        fade,
    })
}

fn build_semantic_state(snapshot: &DashboardSnapshot, mode: GraphColorMode) -> SemanticState {
    let vectors = semantic_vectors(snapshot, mode);
    if vectors.is_empty() {
        return SemanticState::default();
    }

    let coverage = vectors.len();
    let clusters = cluster_vectors(&vectors);
    let resonance_pairs = build_resonance_pairs(&vectors, mode);

    SemanticState {
        clusters,
        resonance_pairs,
        coverage,
    }
}

fn build_field_telemetry(
    snapshot: &DashboardSnapshot,
    mode: GraphColorMode,
    semantic_state: &SemanticState,
) -> FieldTelemetry {
    let mut cluster_counts = HashMap::<usize, usize>::new();
    for cluster in semantic_state.clusters.values() {
        *cluster_counts.entry(*cluster).or_default() += 1;
    }

    let mut top_domains = HashMap::<String, usize>::new();
    let mut total_energy = 0.0f32;
    let mut max_energy = 0.0f32;
    for node in &snapshot.graph_nodes {
        *top_domains.entry(node.domain.clone()).or_default() += 1;
        total_energy += node.food_energy;
        max_energy = max_energy.max(node.food_energy);
    }

    let mut cluster_sizes = cluster_counts
        .into_iter()
        .map(|(cluster_index, count)| ClusterTelemetry {
            cluster_index,
            count,
        })
        .collect::<Vec<_>>();
    cluster_sizes.sort_by(|left, right| right.count.cmp(&left.count));

    let mut top_domains = top_domains
        .into_iter()
        .map(|(domain, count)| DomainTelemetry { domain, count })
        .collect::<Vec<_>>();
    top_domains.sort_by(|left, right| right.count.cmp(&left.count));
    top_domains.truncate(5);

    FieldTelemetry {
        mode,
        total_nodes: snapshot.graph_nodes.len(),
        coverage: semantic_state.coverage,
        resonance_links: semantic_state.resonance_pairs.len(),
        average_energy: if snapshot.graph_nodes.is_empty() {
            0.0
        } else {
            total_energy / snapshot.graph_nodes.len() as f32
        },
        max_energy,
        cluster_sizes,
        top_domains,
    }
}

fn semantic_vectors(snapshot: &DashboardSnapshot, mode: GraphColorMode) -> Vec<(String, Vec<f32>)> {
    snapshot
        .pages
        .iter()
        .filter_map(|page| {
            let vector = match mode {
                GraphColorMode::Novelty => None,
                GraphColorMode::PageSemantic => page.embedding.as_ref(),
                GraphColorMode::JudgmentSemantic => page.llm_embedding.as_ref(),
            }?;

            let normalized = normalize_vector(vector);
            if normalized.is_empty() {
                None
            } else {
                Some((page.url.clone(), normalized))
            }
        })
        .collect()
}

fn apply_semantic_forces(
    semantic_state: &SemanticState,
    url_to_index: &HashMap<&str, usize>,
    positions: &[Vec3],
    forces: &mut [Vec3],
    mode: GraphColorMode,
) {
    let (pair_gain, pair_target, cluster_gain) = match mode {
        GraphColorMode::Novelty => return,
        GraphColorMode::PageSemantic => (0.48, 0.24, 0.065),
        GraphColorMode::JudgmentSemantic => (0.54, 0.22, 0.072),
    };

    for pair in &semantic_state.resonance_pairs {
        let Some(&left) = url_to_index.get(pair.left_url.as_str()) else {
            continue;
        };
        let Some(&right) = url_to_index.get(pair.right_url.as_str()) else {
            continue;
        };

        let delta = positions[right] - positions[left];
        let distance = delta.length().max(0.01);
        let direction = delta / distance;
        let attraction = (distance - pair_target) * pair_gain * pair.similarity;
        forces[left] += direction * attraction;
        forces[right] -= direction * attraction;
    }

    let mut cluster_centers = HashMap::<usize, (Vec3, usize)>::new();
    for (url, cluster) in &semantic_state.clusters {
        let Some(&index) = url_to_index.get(url.as_str()) else {
            continue;
        };
        let entry = cluster_centers.entry(*cluster).or_insert((Vec3::ZERO, 0));
        entry.0 += positions[index];
        entry.1 += 1;
    }

    for (url, cluster) in &semantic_state.clusters {
        let Some(&index) = url_to_index.get(url.as_str()) else {
            continue;
        };
        let Some((center_sum, count)) = cluster_centers.get(cluster) else {
            continue;
        };
        if *count == 0 {
            continue;
        }

        let center = *center_sum / *count as f32;
        let mut anchor = cluster_anchor(*cluster);
        if mode == GraphColorMode::JudgmentSemantic {
            anchor *= 1.18;
        }
        forces[index] += (center + anchor - positions[index]) * cluster_gain;
    }
}

fn cluster_anchor(cluster: usize) -> Vec3 {
    let angle = cluster as f32 * 1.847;
    Vec3::new(angle.cos(), (angle * 0.7).sin(), angle.sin()) * 0.18
}

fn semantic_signature(snapshot: &DashboardSnapshot, mode: GraphColorMode) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    mode.hash(&mut hasher);
    for page in &snapshot.pages {
        page.url.hash(&mut hasher);
        page.content_hash.hash(&mut hasher);
        match mode {
            GraphColorMode::Novelty => {}
            GraphColorMode::PageSemantic => {
                hash_option_embedding(page.embedding.as_deref(), &mut hasher);
            }
            GraphColorMode::JudgmentSemantic => {
                hash_option_embedding(page.llm_embedding.as_deref(), &mut hasher);
                if let Some(llm) = &page.llm_novelty {
                    llm.concise_reason.hash(&mut hasher);
                    llm.recommended_action.hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

fn hash_option_embedding(embedding: Option<&[f32]>, hasher: &mut impl Hasher) {
    if let Some(embedding) = embedding {
        embedding.len().hash(hasher);
        for value in embedding.iter().take(6) {
            value.to_bits().hash(hasher);
        }
    } else {
        0usize.hash(hasher);
    }
}

fn normalize_vector(values: &[f32]) -> Vec<f32> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return Vec::new();
    }

    values.iter().map(|value| value / norm).collect()
}

fn cluster_vectors(vectors: &[(String, Vec<f32>)]) -> HashMap<String, usize> {
    if vectors.is_empty() {
        return HashMap::new();
    }

    if vectors.len() == 1 {
        return HashMap::from([(vectors[0].0.clone(), 0)]);
    }

    let k = ((vectors.len() as f32).sqrt().round() as usize)
        .clamp(2, 6)
        .min(vectors.len());
    let mut sorted = vectors.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    let mut centroids = (0..k)
        .map(|index| {
            let slot = index * sorted.len() / k;
            sorted[slot].1.clone()
        })
        .collect::<Vec<_>>();
    let mut assignments = vec![0usize; vectors.len()];

    for _ in 0..6 {
        for (index, (_, vector)) in vectors.iter().enumerate() {
            assignments[index] = centroids
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| {
                    cosine_similarity(vector, left).total_cmp(&cosine_similarity(vector, right))
                })
                .map(|(centroid_index, _)| centroid_index)
                .unwrap_or(0);
        }

        let dimension = centroids[0].len();
        let mut next = vec![vec![0.0; dimension]; k];
        let mut counts = vec![0usize; k];

        for (assignment, (_, vector)) in assignments.iter().zip(vectors.iter()) {
            counts[*assignment] += 1;
            for (slot, value) in next[*assignment].iter_mut().zip(vector.iter()) {
                *slot += *value;
            }
        }

        for index in 0..k {
            if counts[index] == 0 {
                next[index] = sorted[index % sorted.len()].1.clone();
                continue;
            }

            let scale = 1.0 / counts[index] as f32;
            for value in &mut next[index] {
                *value *= scale;
            }
            next[index] = normalize_vector(&next[index]);
        }

        centroids = next;
    }

    vectors
        .iter()
        .zip(assignments)
        .map(|((url, _), assignment)| (url.clone(), assignment))
        .collect()
}

fn build_resonance_pairs(
    vectors: &[(String, Vec<f32>)],
    mode: GraphColorMode,
) -> Vec<SemanticPair> {
    if vectors.len() < 2 {
        return Vec::new();
    }

    let threshold = match mode {
        GraphColorMode::Novelty => 1.1,
        GraphColorMode::PageSemantic => 0.78,
        GraphColorMode::JudgmentSemantic => 0.82,
    };

    let mut pairs = Vec::new();
    for i in 0..vectors.len() {
        for j in (i + 1)..vectors.len() {
            let similarity = cosine_similarity(&vectors[i].1, &vectors[j].1);
            if similarity < threshold {
                continue;
            }

            pairs.push(SemanticPair {
                left_url: vectors[i].0.clone(),
                right_url: vectors[j].0.clone(),
                similarity,
            });
        }
    }

    pairs.sort_by(|left, right| right.similarity.total_cmp(&left.similarity));
    pairs.truncate(28);
    pairs
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>()
}

fn draw_grid(painter: &egui::Painter, rect: Rect) {
    let grid_color = Color32::from_rgba_unmultiplied(72, 98, 120, 28);
    let scanline = Color32::from_rgba_unmultiplied(255, 72, 72, 12);
    let step = 36.0;

    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, grid_color),
        );
        x += step;
    }

    let mut y = rect.top();
    let mut row = 0;
    while y <= rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, if row % 4 == 0 { scanline } else { grid_color }),
        );
        y += step;
        row += 1;
    }
}

fn draw_scope_rings(painter: &egui::Painter, rect: Rect, mode: GraphColorMode) {
    let center = rect.center();
    let max_radius = rect.width().min(rect.height()) * 0.35;
    let ring_color = match mode {
        GraphColorMode::Novelty => Color32::from_rgba_unmultiplied(255, 96, 96, 18),
        GraphColorMode::PageSemantic => Color32::from_rgba_unmultiplied(92, 225, 255, 22),
        GraphColorMode::JudgmentSemantic => Color32::from_rgba_unmultiplied(255, 176, 72, 24),
    };

    for factor in [0.22, 0.36, 0.52, 0.72, 1.0] {
        painter.circle_stroke(center, max_radius * factor, Stroke::new(1.0, ring_color));
    }
}

fn draw_semantic_resonance(
    painter: &egui::Painter,
    projection_by_url: &HashMap<&str, Projection>,
    semantic_state: &SemanticState,
    mode: GraphColorMode,
) {
    let base = match mode {
        GraphColorMode::Novelty => Color32::from_rgba_unmultiplied(0, 0, 0, 0),
        GraphColorMode::PageSemantic => Color32::from_rgb(92, 225, 255),
        GraphColorMode::JudgmentSemantic => Color32::from_rgb(255, 176, 72),
    };

    for pair in &semantic_state.resonance_pairs {
        let Some(left) = projection_by_url.get(pair.left_url.as_str()) else {
            continue;
        };
        let Some(right) = projection_by_url.get(pair.right_url.as_str()) else {
            continue;
        };

        let alpha = (pair.similarity.clamp(0.0, 1.0) * 120.0) as u8;
        painter.line_segment(
            [left.screen, right.screen],
            Stroke::new(
                1.2 + (pair.similarity - 0.75).max(0.0) * 3.0,
                Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha),
            ),
        );
    }
}

fn node_color(mode: GraphColorMode, energy: f32, fade: f32, cluster: Option<usize>) -> Color32 {
    match mode {
        GraphColorMode::Novelty => energy_color(energy, fade),
        GraphColorMode::PageSemantic => cluster_color(cluster, fade, PAGE_CLUSTER_COLORS),
        GraphColorMode::JudgmentSemantic => cluster_color(cluster, fade, JUDGMENT_CLUSTER_COLORS),
    }
}

fn node_stroke(mode: GraphColorMode, fade: f32, cluster: Option<usize>) -> Stroke {
    let color = match mode {
        GraphColorMode::Novelty => Color32::from_rgba_unmultiplied(255, 92, 92, 90),
        GraphColorMode::PageSemantic => cluster_outline(cluster, fade, PAGE_CLUSTER_COLORS),
        GraphColorMode::JudgmentSemantic => cluster_outline(cluster, fade, JUDGMENT_CLUSTER_COLORS),
    };
    Stroke::new(1.0, color)
}

fn energy_color(energy: f32, fade: f32) -> Color32 {
    let cold = Vec3::new(80.0, 220.0, 255.0);
    let hot = Vec3::new(255.0, 96.0, 96.0);
    let mixed = cold.lerp(hot, energy.clamp(0.0, 1.0));
    Color32::from_rgba_unmultiplied(
        mixed.x as u8,
        mixed.y as u8,
        mixed.z as u8,
        (180.0 * fade + 40.0) as u8,
    )
}

const PAGE_CLUSTER_COLORS: [Color32; 6] = [
    Color32::from_rgb(80, 220, 255),
    Color32::from_rgb(110, 245, 190),
    Color32::from_rgb(196, 108, 255),
    Color32::from_rgb(255, 180, 86),
    Color32::from_rgb(255, 106, 141),
    Color32::from_rgb(182, 255, 106),
];

const JUDGMENT_CLUSTER_COLORS: [Color32; 6] = [
    Color32::from_rgb(255, 166, 72),
    Color32::from_rgb(255, 112, 72),
    Color32::from_rgb(255, 214, 96),
    Color32::from_rgb(255, 93, 138),
    Color32::from_rgb(170, 246, 115),
    Color32::from_rgb(113, 204, 255),
];

fn cluster_color(cluster: Option<usize>, fade: f32, palette: [Color32; 6]) -> Color32 {
    let Some(cluster) = cluster else {
        return Color32::from_rgba_unmultiplied(96, 108, 128, (100.0 * fade + 30.0) as u8);
    };
    let color = palette[cluster % palette.len()];
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (180.0 * fade + 40.0) as u8)
}

fn cluster_outline(cluster: Option<usize>, fade: f32, palette: [Color32; 6]) -> Color32 {
    let Some(cluster) = cluster else {
        return Color32::from_rgba_unmultiplied(124, 136, 156, (90.0 * fade + 24.0) as u8);
    };
    let color = palette[cluster % palette.len()];
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (130.0 * fade + 50.0) as u8)
}

fn draw_tooltip(
    ctx: &egui::Context,
    hovered: HoveredNode<'_>,
    mode: GraphColorMode,
    semantic_state: &SemanticState,
) {
    egui::Area::new(egui::Id::new(("graph-tooltip", &hovered.page.url)))
        .order(egui::Order::Tooltip)
        .fixed_pos(hovered.position)
        .show(ctx, |ui| {
            egui::Frame::default()
                .fill(Color32::from_rgba_unmultiplied(8, 12, 18, 240))
                .stroke(Stroke::new(1.0, Color32::from_rgb(59, 78, 104)))
                .corner_radius(8.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_max_width(360.0);
                    ui.label(
                        egui::RichText::new(&hovered.page.title)
                            .strong()
                            .color(Color32::from_rgb(234, 239, 245)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "energy {:.2}  radius {:.1}",
                            hovered.page.food_energy, hovered.radius
                        ))
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_rgb(116, 205, 255)),
                    );
                    if mode != GraphColorMode::Novelty {
                        let semantic_line = semantic_state
                            .clusters
                            .get(&hovered.page.url)
                            .map(|cluster| format!("{}  cluster {:02}", mode.label(), cluster + 1))
                            .unwrap_or_else(|| format!("{}  no embedding", mode.label()));
                        ui.label(
                            egui::RichText::new(semantic_line)
                                .monospace()
                                .size(11.0)
                                .color(Color32::from_rgb(255, 188, 102)),
                        );
                    }
                    ui.hyperlink_to(
                        egui::RichText::new(&hovered.page.url)
                            .size(11.0)
                            .color(Color32::from_rgb(113, 204, 255)),
                        &hovered.page.url,
                    );
                    ui.add_space(4.0);

                    if let Some(llm) = &hovered.page.llm_novelty {
                        ui.label(
                            egui::RichText::new(format!(
                                "LLM {} | factual {:.2} | entity {:.2} | relational {:.2}",
                                llm.recommended_action,
                                llm.factual_novelty,
                                llm.entity_novelty,
                                llm.relational_novelty
                            ))
                            .monospace()
                            .size(11.0)
                            .color(Color32::from_rgb(255, 93, 93)),
                        );
                        ui.label(
                            egui::RichText::new(&llm.concise_reason)
                                .size(12.0)
                                .color(Color32::from_rgb(212, 220, 229)),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "heuristic score {:.2}",
                                hovered.page.food_energy
                            ))
                            .monospace()
                            .size(11.0)
                            .color(Color32::from_rgb(255, 201, 92)),
                        );
                        ui.label(
                            egui::RichText::new(&hovered.page.reason)
                                .size(12.0)
                                .color(Color32::from_rgb(212, 220, 229)),
                        );
                    }

                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&hovered.page.summary)
                            .size(12.0)
                            .color(Color32::from_rgb(188, 201, 214)),
                    );
                });
        });
}

fn seeded_position(url: &str) -> Vec3 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let seed = hasher.finish();
    let x = ((seed & 0xff) as f32 / 255.0) * 2.0 - 1.0;
    let y = (((seed >> 8) & 0xff) as f32 / 255.0) * 2.0 - 1.0;
    let z = (((seed >> 16) & 0xff) as f32 / 255.0) * 2.0 - 1.0;
    Vec3::new(x, y, z) * 1.3
}

#[cfg(test)]
mod tests {
    use super::{GraphColorMode, build_semantic_state, seeded_position};
    use crate::models::{DashboardSnapshot, GraphEdge, GraphNode, PageRecord};
    use chrono::Utc;

    #[test]
    fn seeded_position_is_stable() {
        assert_eq!(
            seeded_position("https://example.com"),
            seeded_position("https://example.com")
        );
    }

    #[test]
    fn semantic_state_clusters_pages_with_embeddings() {
        let snapshot = DashboardSnapshot {
            total_pages: 2,
            queued_pages: 0,
            failed_pages: 0,
            pages: vec![
                PageRecord {
                    url: "https://a.example".to_string(),
                    domain: "a.example".to_string(),
                    title: "A".to_string(),
                    summary: "A".to_string(),
                    reason: String::new(),
                    food_energy: 0.2,
                    depth: 0,
                    content_hash: "a".to_string(),
                    embedding: Some(vec![1.0, 0.0, 0.0]),
                    llm_embedding: None,
                    llm_novelty: None,
                    created_at: Utc::now(),
                },
                PageRecord {
                    url: "https://b.example".to_string(),
                    domain: "b.example".to_string(),
                    title: "B".to_string(),
                    summary: "B".to_string(),
                    reason: String::new(),
                    food_energy: 0.4,
                    depth: 0,
                    content_hash: "b".to_string(),
                    embedding: Some(vec![0.98, 0.01, 0.0]),
                    llm_embedding: None,
                    llm_novelty: None,
                    created_at: Utc::now(),
                },
            ],
            frontier: Vec::new(),
            graph_nodes: vec![
                GraphNode {
                    url: "https://a.example".to_string(),
                    title: "A".to_string(),
                    domain: "a.example".to_string(),
                    food_energy: 0.2,
                    reason: String::new(),
                },
                GraphNode {
                    url: "https://b.example".to_string(),
                    title: "B".to_string(),
                    domain: "b.example".to_string(),
                    food_energy: 0.4,
                    reason: String::new(),
                },
            ],
            graph_edges: Vec::<GraphEdge>::new(),
        };

        let state = build_semantic_state(&snapshot, GraphColorMode::PageSemantic);
        assert_eq!(state.coverage, 2);
        assert_eq!(state.clusters.len(), 2);
    }
}
