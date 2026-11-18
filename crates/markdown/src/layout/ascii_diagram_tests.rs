use super::*;

const OPEN_TIMELINE: &[&str] = &[
    "Day -29          Day -7           Day -1    Today    Day +1",
    "  │                │                │         │         │",
    "  ├────────────────┼────────────────┼─────────┼─────────┤",
    "  │                 │                │         │         │",
    "  │  30天历史行为   │  近期模式      │ 昨日    │ 今日    │ 次日展示",
    "  │  (服务端存储)   │  (7天细粒度)   │ 增量    │ WPS     │ 焦点项",
    "  │                 │                │ 上传    │ 首页    │",
    "  │                 │                │         │         │",
    "  ▼                 ▼                ▼         ▼         ▼",
];

const ELECTRON_GO_ARCHITECTURE: &[&str] = &[
    "Electron (UI Shell)          Go (Agent Core)",
    "┌──────────────────────┐    ┌─────────────────────────────┐",
    "│  Main Process         │    │  WebSocket Server            │",
    "│  ├─ spawn Go 二进制    │◄──►│  ├─ token 认证               │",
    "│  └─ BrowserWindow     │ WS │  └─ 收发 JSON 消息           │",
    "│                       │    │                              │",
    "│  Renderer (Chat UI)   │    │  Agent Loop                  │",
    "│  ├─ 流式对话           │    │  ├─ Orchestrator (主 agent)  │",
    "│  ├─ 工具调用卡片        │    │  └─ Worker (子 agent)       │",
    "│  ├─ 子 agent 进度      │    │                              │",
    "│  └─ 任务面板            │    │  LLM Provider               │",
    "└──────────────────────┘    │  ├─ Anthropic (流式 SSE)      │",
    "                            │  └─ OpenAI 兼容               │",
    "                            │                               │",
    "                            │  工具系统 (8 tools)            │",
    "                            │  条件式 Prompt 构建            │",
    "                            │  Skills 系统                  │",
    "                            │  会话持久化                    │",
    "                            └─────────────────────────────┘",
];

fn lines(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn detects_open_timeline_without_rectangle_corners() {
    assert!(
        detect_ascii_diagram(&lines(OPEN_TIMELINE)).is_some(),
        "a strongly connected open timeline must use the fixed-grid renderer"
    );
}

#[test]
fn rejects_open_timeline_with_wrong_junction_topology() {
    assert!(detect_ascii_diagram(&lines(&["──┼──┼──", "  │  │"])).is_none());
    assert!(detect_ascii_diagram(&lines(&["├─┬─┤", "│   │"])).is_none());
}

#[test]
fn aligns_open_timeline_tracks_with_cjk_content() {
    const EXPECTED_COLUMNS: [usize; 5] = [2, 20, 37, 47, 57];

    let diagram = detect_ascii_diagram(&lines(OPEN_TIMELINE))
        .expect("a strongly connected open timeline must be detected");

    for row_index in 1..diagram.rows.len() {
        let row = &diagram.rows[row_index];
        let mut member_indices = open_timeline_anchor_indices(row);
        if member_indices.is_empty() {
            member_indices = open_timeline_member_indices(row);
        }
        let render_columns = member_indices
            .iter()
            .map(|cell_index| row.cells[*cell_index].render_column())
            .collect::<Vec<_>>();

        assert_eq!(
            render_columns, EXPECTED_COLUMNS,
            "timeline row {row_index} must use the shared vertical tracks"
        );
    }
}

#[test]
fn leaves_mismatched_open_timeline_row_at_source_columns() {
    let diagram = detect_ascii_diagram(&lines(&["  │     │    │", "  ├────┼────┤", "  │    │"]))
        .expect("the first two rows provide a strongly connected open timeline");
    let mismatched_row = &diagram.rows[2];
    let render_columns = open_timeline_member_indices(mismatched_row)
        .iter()
        .map(|cell_index| mismatched_row.cells[*cell_index].render_column())
        .collect::<Vec<_>>();

    assert_eq!(
        render_columns,
        [2, 7],
        "a row with a different member count must not be guessed into the open tracks"
    );
}

#[test]
fn neighboring_spine_terminates_open_track_scan() {
    let diagram = detect_ascii_diagram(&lines(&["│     │    │", "├────┼────┤", "├──┤  │ │ │"]))
        .expect("the first two rows provide a strongly connected open timeline");
    let neighboring_spine_row = &diagram.rows[2];
    let render_columns = open_timeline_member_indices(neighboring_spine_row)
        .iter()
        .map(|cell_index| neighboring_spine_row.cells[*cell_index].render_column())
        .collect::<Vec<_>>();

    assert_eq!(
        render_columns,
        [6, 8, 10],
        "a neighboring spine must terminate the current open track group"
    );
}

#[test]
fn closed_table_divider_does_not_create_open_tracks() {
    let diagram =
        detect_ascii_diagram(&lines(&["┌─────┐", "│  │  │", "├──┼──┤", "│   │ │", "└─────┘"]))
            .expect("a closed table must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[0], 6).render_column(), 6);
    assert_eq!(cell_at_source_column(&diagram.rows[4], 6).render_column(), 6);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 3);
    assert_eq!(cell_at_source_column(&diagram.rows[3], 4).render_column(), 4);
}

#[test]
fn independent_open_timeline_inside_rectangle_still_aligns() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌─────────────┐",
        "│  │      │   │",
        "│  ├──────┤   │",
        "│  │       │  │",
        "└─────────────┘",
    ]))
    .expect("an outer rectangle containing an independent open timeline must be detected");

    for (row_index, source_column) in [(1, 10), (2, 10), (3, 11)] {
        assert_eq!(
            cell_at_source_column(&diagram.rows[row_index], source_column).render_column(),
            11,
            "inner timeline right track must align on row {row_index}"
        );
    }
    for row_index in 0..diagram.rows.len() {
        assert_eq!(
            right_edge_cell(&diagram.rows[row_index]).render_column(),
            15,
            "outer rectangle must remain aligned after inner track correction on row {row_index}"
        );
    }
}

fn cell_at_source_column(row: &AsciiDiagramRow, column: usize) -> &AsciiDiagramCell {
    row.cells
        .iter()
        .find(|cell| cell.column == column)
        .expect("fixture must contain a cell at the requested source column")
}

fn right_edge_cell(row: &AsciiDiagramRow) -> &AsciiDiagramCell {
    row.cells
        .iter()
        .rev()
        .find(|cell| matches!(cell.text.as_str(), "│" | "┐" | "┘"))
        .expect("fixture row must contain a right edge")
}

#[test]
fn aligns_electron_shell_outer_right_edge_across_tree_branch_rows() {
    const LEFT_FRAME_RIGHT_SOURCES: [(usize, usize); 11] = [
        (1, 23),
        (2, 24),
        (3, 25),
        (4, 24),
        (5, 24),
        (6, 24),
        (7, 25),
        (8, 26),
        (9, 25),
        (10, 26),
        (11, 23),
    ];

    let diagram = detect_ascii_diagram(&lines(ELECTRON_GO_ARCHITECTURE))
        .expect("the architecture fixture must be detected");
    let render_columns = LEFT_FRAME_RIGHT_SOURCES.map(|(row_index, source_column)| {
        cell_at_source_column(&diagram.rows[row_index], source_column).render_column()
    });

    assert_eq!(render_columns, [26; LEFT_FRAME_RIGHT_SOURCES.len()]);
    for row_index in [3, 7, 8, 9] {
        assert_eq!(
            cell_at_source_column(&diagram.rows[row_index], 3).render_column(),
            3,
            "tree branch on row {row_index} must stay at its source column"
        );
    }
}

fn candidate_position_for_assignment(
    assignment: RowTrackAssignment,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
) -> CandidatePosition {
    CandidatePosition {
        expected_position: expected_track_indices
            .iter()
            .position(|track_index| *track_index == assignment.track_index)
            .expect("assignment track must occur in the expected topology"),
        cell_position: cell_indices
            .iter()
            .position(|cell_index| *cell_index == assignment.cell_index)
            .expect("assignment cell must occur in the actual edge sequence"),
    }
}

#[test]
fn aligning_one_edge_shifts_every_following_cell() {
    let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
    let first_right_index = row
        .cells
        .iter()
        .position(|cell| cell.column == 2)
        .expect("fixture contains the first right corner");

    align_row_suffix_to(&mut row, first_right_index, 4);

    assert_eq!(cell_at_source_column(&row, 2).render_column(), 4);
    assert_eq!(cell_at_source_column(&row, 5).render_column(), 7);
    assert_eq!(cell_at_source_column(&row, 7).render_column(), 9);
}

#[test]
fn inherited_shift_does_not_extend_later_horizontal_edges() {
    let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
    let first_right_index = row
        .cells
        .iter()
        .position(|cell| cell.column == 2)
        .expect("fixture contains the first right corner");

    align_row_suffix_to(&mut row, first_right_index, 4);

    assert_eq!(cell_at_source_column(&row, 2).left_extension_columns(), 2);
    assert_eq!(cell_at_source_column(&row, 5).left_extension_columns(), 0);
    assert_eq!(cell_at_source_column(&row, 7).left_extension_columns(), 0);
}

#[test]
fn later_track_uses_columns_shifted_by_the_previous_track() {
    let (first_row, _, _) = grid_row("│  │  │  │");
    let (mut second_row, _, _) = grid_row("│ │  │ │");

    align_row_suffix_to(&mut second_row, 2, 3);
    assert_eq!(cell_at_source_column(&second_row, 5).render_column(), 6);

    align_row_suffix_to(&mut second_row, 5, 7);
    assert_eq!(cell_at_source_column(&second_row, 7).render_column(), 9);
    assert_eq!(cell_at_source_column(&first_row, 9).render_column(), 9);
}

#[test]
fn pairs_rectangle_corners_when_both_sides_drift_between_top_and_bottom() {
    let rows = lines(&[
        "                                                     ┌───────────────────────┐",
        "                                                    │                        │",
        "                                                   └─────────────────────────┘",
    ])
    .iter()
    .map(|line| grid_row(line).0)
    .collect::<Vec<_>>();

    let candidates = rectangle_candidates(&rows);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].top.left_column, 53);
    assert_eq!(candidates[0].top.right_column, 77);
    assert_eq!(candidates[0].bottom.left_column, 51);
    assert_eq!(candidates[0].bottom.right_column, 77);
}

#[test]
fn closest_open_layer_closes_before_its_outer_rectangle() {
    let rows = lines(&["┌─────────┐", "  ┌─────┐", "  └─────┘", "└─────────┘"])
        .iter()
        .map(|line| grid_row(line).0)
        .collect::<Vec<_>>();

    let candidates = rectangle_candidates(&rows);

    assert_eq!(candidates.len(), 2);
    assert_eq!((candidates[0].top.row_index, candidates[0].bottom.row_index), (0, 3));
    assert_eq!((candidates[1].top.row_index, candidates[1].bottom.row_index), (1, 2));
}

#[test]
fn equal_overlap_on_the_closest_open_row_is_ambiguous() {
    let rows = lines(&["┌───┐ ┌───┐", "  └─────┘"])
        .iter()
        .map(|line| grid_row(line).0)
        .collect::<Vec<_>>();

    assert!(rectangle_candidates(&rows).is_empty());
}

#[test]
fn arrow_after_a_shifted_boundary_inherits_the_prefix_delta() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐  ┌──┐", "│  │ →  │  │", "└────┘  └──┘"]))
        .expect("fixture must be detected");

    let shifted_boundary = cell_at_source_column(&diagram.rows[1], 3);
    let arrow = cell_at_source_column(&diagram.rows[1], 5);
    let following_boundary = cell_at_source_column(&diagram.rows[1], 8);

    assert_eq!(shifted_boundary.render_column(), 5);
    assert_eq!(arrow.render_column(), 7);
    assert_eq!(following_boundary.render_column(), 10);
    assert!(shifted_boundary.render_column() < arrow.render_column());
    assert!(arrow.render_column() < following_boundary.render_column());
}

#[test]
fn many_parallel_nested_tracks_keep_only_uniquely_supported_low_confidence_edges() {
    const TRAILING_SPAN_LEFT_COLUMN: usize = 100;
    const UNIQUE_TRAILING_SPAN: usize = 41;
    const TRAILING_SPAN_RIGHT_COLUMN: usize = TRAILING_SPAN_LEFT_COLUMN + UNIQUE_TRAILING_SPAN;
    let actual_columns = [
        0,
        2,
        8,
        10,
        14,
        16,
        22,
        28,
        31,
        37,
        38,
        TRAILING_SPAN_LEFT_COLUMN,
        TRAILING_SPAN_RIGHT_COLUMN,
    ];
    let mut fixture_characters = vec![' '; TRAILING_SPAN_RIGHT_COLUMN + 1];
    for column in actual_columns {
        fixture_characters[column] = '│';
    }
    let fixture = fixture_characters.into_iter().collect::<String>();
    let (row, _, _) = grid_row(&fixture);
    let cell_indices = vertical_edge_cell_indices(&row);
    let tracks = (0..7)
        .flat_map(|rectangle_index| {
            [BorderSide::Left, BorderSide::Right].map(|side| VerticalBorderTrack {
                rectangle_index,
                side,
                members: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    let expected_track_indices = [0, 2, 3, 1, 4, 6, 7, 5, 8, 10, 11, 9, 12, 13];
    let supported_track_columns = [
        (0, 0),
        (2, 2),
        (3, 8),
        (1, 10),
        (4, 14),
        (6, 16),
        (7, 22),
        (5, 24),
        (8, 28),
        (10, 30),
        (11, 36),
        (9, 38),
    ];
    let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
    for (track_index, column) in supported_track_columns {
        supported_columns[track_index].insert(column);
    }
    let mut supported_spans = vec![BTreeSet::new(); 7];
    supported_spans[5].insert(6);
    supported_spans[6].insert(UNIQUE_TRAILING_SPAN);

    let assignments = uniquely_supported_row_assignments(
        &row,
        &expected_track_indices,
        &cell_indices,
        &tracks,
        &supported_columns,
        &supported_spans,
        &SupportedAdjacentGaps::new(),
    );
    let candidates = supported_row_candidates(
        &row,
        &expected_track_indices,
        &cell_indices,
        &tracks,
        &supported_columns,
        &supported_spans,
        &SupportedAdjacentGaps::new(),
    );
    let assignment_positions = assignments
        .iter()
        .copied()
        .map(|assignment| {
            candidate_position_for_assignment(assignment, &expected_track_indices, &cell_indices)
        })
        .collect::<Vec<_>>();
    let assignment_position_set = assignment_positions.iter().copied().collect::<BTreeSet<_>>();

    assert!(candidates.len() <= expected_track_indices.len() * cell_indices.len());
    assert!(assignments.len() <= expected_track_indices.len().min(cell_indices.len()));
    assert_eq!(assignment_position_set.len(), assignments.len());
    assert!(!assignments.is_empty());
    assert!(assignment_positions.windows(2).all(|pair| {
        pair[0].expected_position < pair[1].expected_position
            && pair[0].cell_position < pair[1].cell_position
    }));
    assert!(assignments.iter().all(|assignment| assignment.track_index != 5));

    let mut span_only_assignment_count = 0usize;
    for position in assignment_positions {
        assert_eq!(
            candidates
                .keys()
                .filter(|candidate| { candidate.expected_position == position.expected_position })
                .count(),
            1
        );
        assert_eq!(
            candidates
                .keys()
                .filter(|candidate| candidate.cell_position == position.cell_position)
                .count(),
            1
        );
        assert!(candidates.keys().all(|candidate| {
            *candidate == position || !candidate_positions_cross(position, *candidate)
        }));

        let evidence = &candidates[&position];
        if evidence.directly_supported {
            continue;
        }
        span_only_assignment_count += 1;
        assert!(evidence.atomic_partners.iter().any(|partner| {
            assignment_position_set.contains(partner)
                && candidates.get(partner).is_some_and(|partner_evidence| {
                    partner_evidence.atomic_partners.contains(&position)
                })
        }));
    }
    assert_eq!(span_only_assignment_count, 2);
}

#[test]
fn low_confidence_assignment_has_no_recursive_search_path() {
    let source = include_str!("ascii_diagram_align.rs");
    let forbidden_symbols = [
        ["collect_supported_", "mappings"].concat(),
        ["BestRow", "Assignments"].concat(),
        ["too_many_", "arguments"].concat(),
    ];
    for forbidden_symbol in forbidden_symbols {
        assert!(!source.contains(&forbidden_symbol));
    }

    let function_name = ["uniquely_supported_row_", "assignments"].concat();
    let signature = format!("fn {function_name}(");
    let function_start =
        source.find(&signature).expect("low-confidence assignment function must exist");
    let function_end_marker =
        "\n}\n\npub(in crate::layout::ascii_diagram) fn record_uniquely_supported_row";
    let function_end = source[function_start..]
        .find(function_end_marker)
        .map(|relative_end| function_start + relative_end)
        .expect("low-confidence assignment function must have a bounded source body");
    let function_source = &source[function_start..function_end];

    assert_eq!(function_source.matches(&function_name).count(), 1);
}

#[test]
fn unique_candidate_crossing_ambiguous_candidates_remains_unassigned() {
    let (row, _, _) = grid_row("│││");
    let cell_indices = vertical_edge_cell_indices(&row);
    let tracks = (0..2)
        .map(|rectangle_index| VerticalBorderTrack {
            rectangle_index,
            side: BorderSide::Left,
            members: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
    supported_columns[0].extend([1, 2]);
    supported_columns[1].insert(0);

    let assignments = uniquely_supported_row_assignments(
        &row,
        &[0, 1],
        &cell_indices,
        &tracks,
        &supported_columns,
        &[BTreeSet::new(), BTreeSet::new()],
        &SupportedAdjacentGaps::new(),
    );

    assert!(assignments.is_empty());
}

#[test]
fn adjacent_gap_candidates_are_scoped_to_their_track_pair() {
    let (row, _, _) = grid_row("├    │    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let expected_track_indices = [0, 1, 2];
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 1, right_track_index: 2 },
        BTreeSet::from([5]),
    )]);
    let mut candidates = BTreeMap::new();

    add_adjacent_gap_supported_candidates(
        &row,
        &expected_track_indices,
        &cell_indices,
        &supported_adjacent_gaps,
        &mut candidates,
    );

    let left = CandidatePosition { expected_position: 1, cell_position: 0 };
    let right = CandidatePosition { expected_position: 2, cell_position: 1 };
    assert!(candidates[&left].atomic_partners.contains(&right));
    assert!(candidates[&right].atomic_partners.contains(&left));
    assert!(candidates.keys().all(|position| position.expected_position != 0));
}

#[test]
fn adjacent_gap_candidates_ignore_rows_without_tree_branch_junction() {
    let (row, _, _) = grid_row("│    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 0, right_track_index: 1 },
        BTreeSet::from([5]),
    )]);
    let mut candidates = BTreeMap::new();

    add_adjacent_gap_supported_candidates(
        &row,
        &[0, 1],
        &cell_indices,
        &supported_adjacent_gaps,
        &mut candidates,
    );

    assert!(candidates.is_empty());
}

#[test]
fn adjacent_gap_candidates_reject_endpoints_conflicting_with_direct_support() {
    let (row, _, _) = grid_row(ELECTRON_GO_ARCHITECTURE[3]);
    let cell_indices = vertical_edge_cell_indices(&row);
    let expected_track_indices = [0, 1, 2, 3];
    let supported_adjacent_gaps = BTreeMap::from([
        (AdjacentTrackPair { left_track_index: 1, right_track_index: 2 }, BTreeSet::from([5])),
        (
            AdjacentTrackPair { left_track_index: 2, right_track_index: 3 },
            BTreeSet::from([30, 31, 33]),
        ),
    ]);
    let mut candidates = BTreeMap::from([
        (
            CandidatePosition { expected_position: 0, cell_position: 0 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
        (
            CandidatePosition { expected_position: 3, cell_position: 5 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
    ]);

    add_adjacent_gap_supported_candidates(
        &row,
        &expected_track_indices,
        &cell_indices,
        &supported_adjacent_gaps,
        &mut candidates,
    );

    let target_left = CandidatePosition { expected_position: 1, cell_position: 2 };
    let target_right = CandidatePosition { expected_position: 2, cell_position: 3 };
    let right_boundary = CandidatePosition { expected_position: 3, cell_position: 5 };
    assert!(candidates[&target_left].atomic_partners.contains(&target_right));
    assert!(candidates[&target_right].atomic_partners.contains(&target_left));
    assert!(candidates[&target_right].atomic_partners.contains(&right_boundary));
    assert!(candidates[&right_boundary].atomic_partners.contains(&target_right));
    assert!(
        !candidates.contains_key(&CandidatePosition { expected_position: 2, cell_position: 0 })
    );
    assert!(
        !candidates.contains_key(&CandidatePosition { expected_position: 2, cell_position: 1 })
    );
}

#[test]
fn tree_branch_span_filter_requires_a_tree_branch_junction() {
    let (tree_row, _, _) = grid_row(ELECTRON_GO_ARCHITECTURE[3]);
    let tree_cell_indices = vertical_edge_cell_indices(&tree_row);
    let (plain_row, _, _) = grid_row("│    │    │    │");
    let plain_cell_indices = vertical_edge_cell_indices(&plain_row);
    let expected_track_indices = [0, 1, 2, 3];
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 1, right_track_index: 2 },
        BTreeSet::from([5]),
    )]);
    let tree_candidates = BTreeMap::from([
        (
            CandidatePosition { expected_position: 0, cell_position: 0 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
        (
            CandidatePosition { expected_position: 3, cell_position: 5 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
    ]);
    let plain_candidates = BTreeMap::from([
        (
            CandidatePosition { expected_position: 0, cell_position: 0 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
        (
            CandidatePosition { expected_position: 3, cell_position: 3 },
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
    ]);

    assert!(should_exclude_tree_branch_conflicting_span_candidate(
        &tree_row,
        &expected_track_indices,
        &tree_cell_indices,
        CandidatePosition { expected_position: 2, cell_position: 0 },
        CandidatePosition { expected_position: 3, cell_position: 3 },
        &supported_adjacent_gaps,
        &tree_candidates,
    ));
    assert!(!should_exclude_tree_branch_conflicting_span_candidate(
        &plain_row,
        &expected_track_indices,
        &plain_cell_indices,
        CandidatePosition { expected_position: 2, cell_position: 0 },
        CandidatePosition { expected_position: 3, cell_position: 2 },
        &supported_adjacent_gaps,
        &plain_candidates,
    ));
}

#[test]
fn ambiguous_adjacent_gap_pairs_remain_unassigned() {
    let (row, _, _) = grid_row("├    │    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let tracks = [BorderSide::Left, BorderSide::Right]
        .map(|side| VerticalBorderTrack { rectangle_index: 0, side, members: Vec::new() })
        .to_vec();
    let supported_adjacent_gaps = BTreeMap::from([(
        AdjacentTrackPair { left_track_index: 0, right_track_index: 1 },
        BTreeSet::from([5]),
    )]);
    let mut candidates = BTreeMap::new();

    add_adjacent_gap_supported_candidates(
        &row,
        &[0, 1],
        &cell_indices,
        &supported_adjacent_gaps,
        &mut candidates,
    );
    assert!(
        !candidates.is_empty(),
        "tree branch row must generate the equal-gap candidates under test"
    );

    let assignments = uniquely_supported_row_assignments(
        &row,
        &[0, 1],
        &cell_indices,
        &tracks,
        &[BTreeSet::new(), BTreeSet::new()],
        &[BTreeSet::new()],
        &supported_adjacent_gaps,
    );

    assert!(assignments.is_empty(), "two equal gap pairs must remain ambiguous");
}

#[test]
fn direct_candidate_conflicting_with_a_complete_span_pair_remains_unassigned() {
    let (row, _, _) = grid_row("│    │    │");
    let cell_indices = vertical_edge_cell_indices(&row);
    let tracks = [BorderSide::Left, BorderSide::Right]
        .map(|side| VerticalBorderTrack { rectangle_index: 0, side, members: Vec::new() })
        .to_vec();
    let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
    supported_columns[0].insert(5);
    let mut supported_spans = vec![BTreeSet::new()];
    supported_spans[0].insert(10);

    let assignments = uniquely_supported_row_assignments(
        &row,
        &[0, 1],
        &cell_indices,
        &tracks,
        &supported_columns,
        &supported_spans,
        &SupportedAdjacentGaps::new(),
    );

    assert!(assignments.is_empty());
}

#[test]
fn direct_last_boundary_anchor_recovers_its_only_reciprocal_atomic_partner() {
    let partner = CandidatePosition { expected_position: 1, cell_position: 1 };
    let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
    let candidates = BTreeMap::from([
        (
            partner,
            AssignmentEvidence {
                directly_supported: false,
                atomic_partners: BTreeSet::from([boundary]),
            },
        ),
        (
            boundary,
            AssignmentEvidence {
                directly_supported: true,
                atomic_partners: BTreeSet::from([partner]),
            },
        ),
    ]);
    let mut selected = BTreeSet::new();

    include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

    assert_eq!(selected, BTreeSet::from([partner, boundary]));
}

#[test]
fn direct_boundary_anchor_does_not_choose_between_multiple_atomic_partners() {
    let first_partner = CandidatePosition { expected_position: 0, cell_position: 0 };
    let second_partner = CandidatePosition { expected_position: 1, cell_position: 1 };
    let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
    let candidates = BTreeMap::from([
        (
            first_partner,
            AssignmentEvidence {
                directly_supported: false,
                atomic_partners: BTreeSet::from([boundary]),
            },
        ),
        (
            second_partner,
            AssignmentEvidence {
                directly_supported: false,
                atomic_partners: BTreeSet::from([boundary]),
            },
        ),
        (
            boundary,
            AssignmentEvidence {
                directly_supported: true,
                atomic_partners: BTreeSet::from([first_partner, second_partner]),
            },
        ),
    ]);
    let mut selected = BTreeSet::new();

    include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

    assert_eq!(selected, BTreeSet::from([boundary]));
}

#[test]
fn direct_boundary_anchor_rejects_a_direct_candidate_sharing_its_actual_edge() {
    let competing = CandidatePosition { expected_position: 2, cell_position: 2 };
    let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
    let candidates = BTreeMap::from([
        (
            competing,
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
        (
            boundary,
            AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
        ),
    ]);
    let mut selected = BTreeSet::new();

    include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

    assert!(selected.is_empty());
}

#[test]
fn boundary_atomic_partner_rejects_crossing_candidate_that_remains_viable() {
    let crossing = CandidatePosition { expected_position: 2, cell_position: 0 };
    let partner = CandidatePosition { expected_position: 1, cell_position: 1 };
    let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
    let candidates = BTreeMap::from([
        (crossing, AssignmentEvidence::default()),
        (
            partner,
            AssignmentEvidence {
                directly_supported: false,
                atomic_partners: BTreeSet::from([boundary]),
            },
        ),
        (
            boundary,
            AssignmentEvidence {
                directly_supported: true,
                atomic_partners: BTreeSet::from([partner]),
            },
        ),
    ]);
    let mut selected = BTreeSet::new();

    include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

    assert_eq!(selected, BTreeSet::from([boundary]));
}

#[test]
fn aligns_wps_architecture_outer_edges_with_nested_and_missing_segments() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌──────────── WPS 客户端 ────────────┐          ┌────── 服务端（增量持续）──────┐",
        "│                                     │          │                              │",
        "│  ┌─ 本地日志（30天滚动）─────────┐  │  每日增量 │  ┌─ 画像更新 Pipeline ───┐  │",
        "│  │ · 文件操作（打开/关闭/保存）  │  │  上传     │  │ · 角色标签更新          │  │",
        "│  │ · 编辑会话（时长/段落/光标）  │  │────────→ │  │ · 模板偏好更新          │  │",
        "│  │ · 模板使用记录               │  │  DSL包   │  │ · 活跃项目检测          │  │",
        "│  │ · 搜索/崩溃日志              │  │ (仅增量)  │  │ · 周期性模式识别        │  │",
        "│  │ · 过去30天全部可触达         │  │          │  └─────────────────────────┘  │",
        "│  └──────────────────────────────┘  │          │                              │",
        "│                                     │          │  ┌─ 行为预测 Agent ──────┐  │",
        "│  ┌─ 本地 Tool（核心）────────────┐  │          │  │ · 读取 30 天滚动窗口    │  │",
        "│  │ ① 扫描 30 天日志目录         │  │          │  │ · 加载用户画像          │  │",
        "│  │ ② 数据清洗 + 去噪 + 聚合    │  │          │  │ · LLM 推理：            │  │",
        "│  │ ③ 结构化 DSL 事件            │  │          │  │   短期意图(续编/模板)    │  │",
        "│  │ ④ 上下文字段填充             │  │          │  │   周期性模式(月报/周报)  │  │",
        "│  │ ⑤ 画像缓存附加               │  │          │  │ · 生成焦点项（≤5 条）   │  │",
        "│  │ ⑥ 增量打包上传               │  │          │  │ · 输出结构化焦点 DSL    │  │",
        "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
        "│                                     │          │                              │",
        "│  ┌─ 焦点渲染 ──────────────────┐  │  每日拉取  │  ┌─ 焦点项存储 + 下发 ───┐  │",
        "│  │ · 首页焦点卡片列表           │  │←──────── │  │ · 存储到用户维度        │  │",
        "│  │ · 排序展示（按优先级）       │  │ 焦点DSL   │  │ · 客户端启动时拉取      │  │",
        "│  │ · 点击执行 action            │  │          │  │ · 附带更新后画像        │  │",
        "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
        "│                                     │          │                              │",
        "│  ┌─ 反馈采集 ──────────────────┐  │          │  ┌─ 反馈回收 ─────────────┐  │",
        "│  │ · 卡片点击（accepted）       │──反馈上报→│  │ · 焦点项点击/忽略统计   │  │",
        "│  │ · 卡片关闭（dismissed）      │  │          │  │ · 纳入画像调整信号      │  │",
        "│  │ · 无操作（ignored）          │  │          │  │ · 周期准确性验证        │  │",
        "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
        "└────────────────────────────────────┘          └──────────────────────────────┘",
    ]))
    .expect("fixture must be detected");

    let client_right_columns = [(0, 37), (1, 38), (12, 36), (30, 37)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert!(client_right_columns.windows(2).all(|pair| pair[0] == pair[1]));

    let server_left_columns = [(0, 48), (3, 50), (12, 47), (19, 49)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert!(server_left_columns.windows(2).all(|pair| pair[0] == pair[1]));

    let server_right_columns = [(0, 80), (3, 82), (12, 79), (30, 79)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert!(server_right_columns.windows(2).all(|pair| pair[0] == pair[1]));

    assert_eq!(cell_at_source_column(&diagram.rows[26], 46).render_column(), 51);
    assert_eq!(cell_at_source_column(&diagram.rows[26], 78).render_column(), 84);
}

#[test]
fn aligns_wps_rolling_window_box_to_its_rightmost_existing_edge() {
    let diagram = detect_ascii_diagram(&lines(&[
        "时间轴（30天滚动窗口）",
        "",
        "Day -29          Day -7           Day -1    Today    Day +1",
        "  │                │                │         │         │",
        "  ├────────────────┼────────────────┼─────────┼─────────┤",
        "  │                 │                │         │         │",
        "  │  30天历史行为   │  近期模式      │ 昨日    │ 今日    │ 次日展示",
        "  │  (服务端存储)   │  (7天细粒度)   │ 增量    │ WPS     │ 焦点项",
        "  │                 │                │ 上传    │ 首页    │",
        "  │                 │                │         │         │",
        "  ▼                 ▼                ▼         ▼         ▼",
        " ┌─────────────────────────────────────────────────────────┐",
        " │  服务端滚动窗口（始终保留最近 30 天行为摘要）             │",
        " │                                                         │",
        " │  每天凌晨：                                              │",
        " │    ① 接收昨日增量 DSL 包 → 追加到滚动窗口               │",
        " │    ② 淘汰 Day-31 旧数据                                 │",
        " │    ③ 更新画像（5个维度全量重算）                        │",
        " │    ④ 短期意图预测（基于 1-7 天近期行为）                │",
        " │    ⑤ 周期性模式检测（基于 7-30 天中长周期）             │",
        " │    ⑥ 合并焦点项 → 排序 → 存储                          │",
        " └─────────────────────────────────────────────────────────┘",
    ]))
    .expect("fixture must be detected");

    let right_columns = (11..=20)
        .map(|row_index| right_edge_cell(&diagram.rows[row_index]).render_column())
        .collect::<Vec<_>>();
    assert!(right_columns.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn missing_outer_side_does_not_claim_an_inner_adjacent_vertical_edge() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌───────────┐",
        "│ ┌─────┐   │",
        "│ │ │   │ │",
        "│ └─────┘   │",
        "└───────────┘",
    ]))
    .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[2], 10).render_column(), 10);
}

#[test]
fn aligns_misaligned_complete_rectangle_right_edges() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌─ 本地日志（30天滚动）─────────┐",
        "│ · 文件操作（打开/关闭/保存）  │",
        "│ · 模板使用记录               │",
        "└──────────────────────────────┘",
    ]))
    .expect("fixture must be detected as an ASCII diagram");

    let source_columns =
        diagram.rows.iter().map(|row| right_edge_cell(row).column).collect::<Vec<_>>();
    let render_columns =
        diagram.rows.iter().map(|row| right_edge_cell(row).render_column()).collect::<Vec<_>>();

    assert_eq!(source_columns, vec![32, 32, 31, 31]);
    assert_eq!(render_columns, vec![32, 32, 32, 32]);
}

#[test]
fn nested_tracks_recompute_outer_target_after_inner_suffix_shift() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌──────────┐",
        "│ ┌──────┐ │",
        "│ │ x  │   │",
        "│ └─────┘  │",
        "└─────────┘",
    ]))
    .expect("fixture must be detected");

    let inner_right_columns = [(1, 9), (2, 7), (3, 8)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert_eq!(inner_right_columns, [9, 9, 9]);

    let outer_right_columns = [(0, 11), (1, 11), (2, 11), (3, 11), (4, 10)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert_eq!(outer_right_columns, [13, 13, 13, 13, 13]);
}

#[test]
fn aligns_discontinuous_rectangle_to_rightmost_existing_edge() {
    let diagram =
        detect_ascii_diagram(&lines(&["┌────┐", "│ a    │", "│       ", "│ c  │", "└────┘"]))
            .expect("fixture must be detected");

    let top_right = cell_at_source_column(&diagram.rows[0], 5);
    let first_existing_side = cell_at_source_column(&diagram.rows[1], 7);
    let second_existing_side = cell_at_source_column(&diagram.rows[3], 5);
    let bottom_right = cell_at_source_column(&diagram.rows[4], 5);

    assert_eq!(top_right.render_column(), 7);
    assert_eq!(first_existing_side.render_column(), 7);
    assert!(is_vertical_edge(cell_at_source_column(&diagram.rows[2], 0)));
    assert_eq!(second_existing_side.render_column(), 7);
    assert_eq!(bottom_right.render_column(), 7);
}

#[test]
fn leaves_ambiguous_discontinuous_right_edges_at_source_columns() {
    let diagram = detect_ascii_diagram(&lines(&["┌─────┐", "│ a  │ │", "└─────┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 7).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[0], 6).render_column(), 6);
}

#[test]
fn missing_inner_side_does_not_claim_the_outer_right_edge() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌──────────┐",
        "│ ┌────┐   │",
        "│ │ x      │",
        "│ └────┘   │",
        "└──────────┘",
    ]))
    .expect("fixture must be detected");

    let only_right_edge_on_gap_row = right_edge_cell(&diagram.rows[2]);
    assert_eq!(only_right_edge_on_gap_row.column, 11);
    assert_eq!(only_right_edge_on_gap_row.render_column(), 11);
    assert_eq!(diagram.rows[2].cells.iter().filter(|cell| is_vertical_edge(cell)).count(), 3);
}

#[test]
fn missing_left_side_does_not_claim_an_unrelated_right_edge() {
    let diagram = detect_ascii_diagram(&lines(&[
        "  ┌────┐",
        "  │ a  │",
        "          │",
        "  │ c  │",
        "  └────┘",
    ]))
    .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[0], 7).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 7).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[2], 10).render_column(), 10);
    assert_eq!(cell_at_source_column(&diagram.rows[3], 7).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[4], 7).render_column(), 7);
}

#[test]
fn parallel_rectangle_tracks_accumulate_from_left_to_right() {
    let diagram =
        detect_ascii_diagram(&lines(&["┌────┐  ┌──────┐", "│a │    │b   │", "└───┘   └─────┘"]))
            .expect("fixture must be detected");

    let left_right_columns = [(0, 5), (1, 3), (2, 4)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert_eq!(left_right_columns, [5, 5, 5]);

    let right_left_columns = [(0, 8), (1, 8), (2, 8)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert_eq!(right_left_columns, [10, 10, 10]);

    let right_right_columns = [(0, 15), (1, 13), (2, 14)]
        .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
    assert_eq!(right_right_columns, [17, 17, 17]);
}

#[test]
fn leaves_incomplete_rectangle_but_aligns_closed_rectangle_suffixes() {
    let incomplete = detect_ascii_diagram(&lines(&["┌────┐", "│x  │"]))
        .expect("thresholds still identify the incomplete fixture as a diagram");
    assert_eq!(right_edge_cell(&incomplete.rows[1]).render_column(), 4);

    let sequential_bottoms = detect_ascii_diagram(&lines(&["┌────┐", "│x │", "└──┘", "└───┘"]))
        .expect("thresholds still identify the closed fixture as a diagram");
    assert_eq!(right_edge_cell(&sequential_bottoms.rows[1]).render_column(), 5);

    let row_with_text = detect_ascii_diagram(&lines(&["┌────┐", "│a │x", "└───┘"]))
        .expect("fixture must be detected");
    assert_eq!(cell_at_source_column(&row_with_text.rows[1], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&row_with_text.rows[1], 4).render_column(), 6);
}

#[test]
fn shifts_horizontal_edge_in_the_row_suffix() {
    let horizontal_gap = detect_ascii_diagram(&lines(&["┌──┐─", "│  │", "└────┘"]))
        .expect("fixture must be detected");
    assert_eq!(cell_at_source_column(&horizontal_gap.rows[0], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&horizontal_gap.rows[0], 4).render_column(), 6);
    assert_eq!(cell_at_source_column(&horizontal_gap.rows[1], 3).render_column(), 5);
}

#[test]
fn shifts_text_after_a_moved_corner() {
    let target_text = detect_ascii_diagram(&lines(&["┌──┐ X", "│  │", "└────┘"]))
        .expect("fixture must be detected");
    assert_eq!(cell_at_source_column(&target_text.rows[0], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&target_text.rows[0], 5).render_column(), 7);
    assert_eq!(cell_at_source_column(&target_text.rows[1], 3).render_column(), 5);
}

#[test]
fn aligns_right_edges_when_the_row_suffix_is_whitespace() {
    let whitespace_path = detect_ascii_diagram(&lines(&["┌──┐  ", "│  │", "└────┘"]))
        .expect("fixture must be detected");
    assert_eq!(cell_at_source_column(&whitespace_path.rows[0], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&whitespace_path.rows[1], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&whitespace_path.rows[2], 5).render_column(), 5);
}

#[test]
fn shifts_text_after_a_moved_middle_edge() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │ X", "└────┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 7);
}

#[test]
fn shifts_box_character_after_a_moved_middle_edge() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │ ─", "└────┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 7);
}

#[test]
fn aligns_middle_edge_to_blank_target_column() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │  ", "└────┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
}

#[test]
fn keeps_middle_edge_at_matching_target_column() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│    │", "└────┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
}

#[test]
fn keeps_vertically_stacked_rectangles_and_arrow_at_source_columns() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌────┐",
        "│ a  │",
        "└────┘",
        "  ↓",
        "┌────┐",
        "│ b  │",
        "└────┘",
    ]))
    .expect("fixture must be detected");

    for row_index in [0, 1, 2, 4, 5, 6] {
        let right_edge = right_edge_cell(&diagram.rows[row_index]);
        assert_eq!(right_edge.render_column(), right_edge.column);
    }
    let arrow = cell_at_source_column(&diagram.rows[3], 2);
    assert_eq!(arrow.text, "↓");
    assert_eq!(arrow.render_column(), arrow.column);
}

#[test]
fn aligns_corner_tracks_when_outer_rectangle_lacks_middle_edges() {
    let diagram = detect_ascii_diagram(&lines(&[
        "┌─────────┐",
        "  ┌─────┐",
        "  │ x  │",
        "  └────┘",
        "└────────┘",
    ]))
    .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[2], 7).render_column(), 8);
    assert_eq!(cell_at_source_column(&diagram.rows[0], 10).render_column(), 10);
    assert_eq!(cell_at_source_column(&diagram.rows[4], 9).render_column(), 10);
}

#[test]
fn aligns_uniquely_supported_shared_right_edges() {
    let diagram = detect_ascii_diagram(&lines(&["┌─┌────┐──┐", "│ │x │", "│ │y  │", "└─└───┘─┘"]))
        .expect("fixture must be detected");

    assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
    assert_eq!(cell_at_source_column(&diagram.rows[2], 6).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[3], 6).render_column(), 7);
    assert_eq!(cell_at_source_column(&diagram.rows[0], 10).render_column(), 10);
    assert_eq!(cell_at_source_column(&diagram.rows[3], 8).render_column(), 10);
}

#[test]
fn detects_light_box_diagram_with_cjk_label() {
    let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│中文│", "└────┘"]))
        .expect("a multi-line light box must be detected");

    assert_eq!(diagram.rows.len(), 3);
    assert_eq!(diagram.column_count, 6);
    assert!(diagram.rows.iter().all(|row| row.column_count == 6));
    assert_eq!(diagram.rows[1].cells[1].column_width, 2);
}

#[test]
fn assigns_zero_columns_to_standalone_combining_mark_graphemes() {
    assert_eq!(grapheme_column_width("\u{0301}"), 0);
}

#[test]
fn assigns_complete_east_asian_widths_to_graphemes() {
    assert_eq!(grapheme_column_width("A"), 1);
    assert_eq!(grapheme_column_width("｟"), 2);
    assert_eq!(grapheme_column_width("￦"), 2);
    assert_eq!(grapheme_column_width("ᄀ"), 2);
    assert_eq!(grapheme_column_width("e\u{0301}"), 1);
    assert_eq!(grapheme_column_width("\u{0301}"), 0);
}

#[test]
fn enforces_box_drawing_detection_threshold_boundaries() {
    assert!(detect_ascii_diagram(&lines(&["┌─┐", "│  │"])).is_none());
    assert!(detect_ascii_diagram(&lines(&["┌────┐", "plain text"])).is_none());
    assert!(detect_ascii_diagram(&lines(&["┌─┐", "└─┘"])).is_some());
}

#[test]
fn preserves_unknown_box_drawing_character_as_text_cell() {
    let diagram =
        detect_ascii_diagram(&lines(&["┌──┐", "│═ │", "└──┘"])).expect("fixture is a diagram");
    let unknown_cell = diagram.rows[1]
        .cells
        .iter()
        .find(|cell| cell.text == "═")
        .expect("fixture contains unknown box drawing character");

    assert_eq!(unknown_cell.box_connections, None);
    assert_eq!(unknown_cell.column_width, 1);
}

#[test]
fn rejects_normal_code_and_single_box_character() {
    assert!(
        detect_ascii_diagram(&lines(&["let result = value + 1;", "println!(\"{result}\");"]))
            .is_none()
    );
    assert!(detect_ascii_diagram(&lines(&["┌ value", "plain text"])).is_none());
}

#[test]
fn maps_all_supported_light_box_connections() {
    let expected_connections = [
        ("─", BoxConnections::LEFT_RIGHT),
        ("│", BoxConnections::UP_DOWN),
        ("┌", BoxConnections::RIGHT_DOWN),
        ("┐", BoxConnections { left: true, down: true, ..BoxConnections::default() }),
        ("└", BoxConnections { right: true, up: true, ..BoxConnections::default() }),
        ("┘", BoxConnections { left: true, up: true, ..BoxConnections::default() }),
        ("├", BoxConnections { right: true, up: true, down: true, left: false }),
        ("┤", BoxConnections { left: true, up: true, down: true, right: false }),
        ("┬", BoxConnections { left: true, right: true, down: true, up: false }),
        ("┴", BoxConnections { left: true, right: true, up: true, down: false }),
        ("┼", BoxConnections::ALL),
    ];

    for (character, expected) in expected_connections {
        assert_eq!(
            box_connections(character),
            Some(expected),
            "unexpected connections for {character}"
        );
    }
}
