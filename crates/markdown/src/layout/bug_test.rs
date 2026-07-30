#[test]
fn test_list_item_click_bug_nested() {
    let md = "- item 1\n  \n  para 2\n\n---";
    let parsed = crate::parser::parse_markdown(md);
    let style = crate::test_utils::default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let view = core::document::StringDocView::new(md);
    let mut lazy = LazyLayout::from_doc(doc.clone(), &style, 400.0, &view);
    
    let mut shaper = shaping::Shaper::new().unwrap();
    
    lazy.ensure_precise_range(0.0, 1000.0, &style, &mut shaper, None, &view);
    println!("After pass 1 total_height: {}, y_delta: {:?}", lazy.total_height, lazy.y_delta);
    
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: 3,
        preedit_text: None,
        preedit_cursor: None,
    }));
    lazy.invalidate_lines_for_source_bytes(vec![3]);
    lazy.ensure_precise_range(0.0, 1000.0, &style, &mut shaper, None, &view);
    println!("After pass 2 total_height: {}, y_delta: {:?}", lazy.total_height, lazy.y_delta);
    
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte: 4,
        preedit_text: None,
        preedit_cursor: None,
    }));
    lazy.invalidate_lines_for_source_bytes(vec![3, 4]);
    lazy.ensure_precise_range(0.0, 1000.0, &style, &mut shaper, None, &view);
    println!("After pass 3 total_height: {}, y_delta: {:?}", lazy.total_height, lazy.y_delta);
}
