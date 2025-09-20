use crate::app::App;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Wrap,
    },
    Terminal,
};
use std::io::Stdout;
use unicode_width::UnicodeWidthStr;

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;
const ANIMATION_FRAMES: &[char] = &['⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn draw_ui(app: &mut App, terminal: &mut TuiTerminal) -> anyhow::Result<()> {
    app.animation_frame = app.animation_frame.wrapping_add(1);
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(5),    // 消息窗口
                Constraint::Length(3), // 输入框
            ])
            .split(f.size());

        // ========== 聊天消息区域 ==========
        let mut text_lines: Vec<Line> = Vec::new();
        for (i, msg) in app.messages.iter().enumerate() {
            // 为不同发件人设置不同的蓝色系样式
            let sender_style = match msg.sender.as_str() {
                "AI" => Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            };
            let sender = Span::styled(format!("{}: ", msg.sender), sender_style);
            let mut content = render_markdown(&msg.content);

            if let Some(first_line) = content.lines.get_mut(0) {
                let mut spans = vec![sender];
                spans.extend(first_line.spans.drain(..));
                first_line.spans = spans;
            } else {
                content.lines.insert(0, Line::from(sender));
            }

            text_lines.extend(content.lines);
            if i < app.messages.len() - 1 {
                text_lines.push(Line::from("")); // 消息间添加空行
            }
        }

        // ========== 流式 AI 消息 ==========
        if app.is_streaming {
            if !text_lines.is_empty() {
                text_lines.push(Line::from(""));
            }
            let frame = ANIMATION_FRAMES[app.animation_frame % ANIMATION_FRAMES.len()];
            let sender = Span::styled(
                format!("AI: {} ", frame),
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            );
            let mut content = render_markdown(&app.current_ai_message);
            if let Some(first_line) = content.lines.get_mut(0) {
                let mut spans = vec![sender];
                spans.extend(first_line.spans.drain(..));
                first_line.spans = spans;
            } else {
                content.lines.insert(0, Line::from(sender));
            }
            text_lines.extend(content.lines);
        }

        let total_lines = text_lines.len();
        let messages_widget = Paragraph::new(text_lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "🌊 Sui Chat",
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded) // 使用圆角边框，更柔和
                    .border_style(Style::default().fg(Color::Blue)), // 边框颜色
            )
            .wrap(Wrap { trim: true })
            .scroll((app.scroll, 0));
        f.render_widget(messages_widget, chunks[0]);

        // ========== 滚动条 - 设计成“水流”样式 ==========
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(app.scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("💧") // 滚动条滑块用“水滴”符号
                .track_symbol(Some("≈")), // 轨道用波浪线
            chunks[0],
            &mut scrollbar_state,
        );

        // ========== 输入框区域 ==========
        let mut input_spans = vec![Span::styled(
            app.input.as_str(),
            Style::default().fg(Color::Yellow),
        )];
        if !app.suggestion.is_empty() {
            input_spans.push(Span::styled(
                app.suggestion.as_str(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        let input_line = Line::from(input_spans);

        let input_widget = Paragraph::new(input_line).block(
            Block::default()
                .title(Span::styled(
                    "Flow your thoughts...",
                    Style::default().fg(Color::Cyan),
                )) // 更具引导性的标题
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        );
        f.render_widget(input_widget, chunks[1]);

        // ========== 光标 - 增加“水波纹”动画效果 ==========
        let cursor_x = chunks[1].x + app.input.width() as u16 + 1;
        let cursor_y = chunks[1].y + 1;
        f.set_cursor(cursor_x, cursor_y);

        // 如果正在流式输出，我们在光标后渲染一个淡蓝色并带有慢速闪烁的 "~" 符号，模拟水波纹
        if app.is_streaming {
            let size = f.size();
            if cursor_x + 1 < size.width {
                let ripple_span = Span::styled(
                    "~",
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::SLOW_BLINK),
                );
                let ripple_rect =
                    ratatui::layout::Rect::new((cursor_x + 1) as u16, cursor_y as u16, 1, 1);
                f.render_widget(Paragraph::new(ripple_span), ripple_rect);
            }
        }
    })?;
    Ok(())
}

fn render_markdown(md_text: &str) -> Text {
    let mut lines = Vec::new();
    let mut current_line_spans = Vec::new();
    let mut style_stack = Vec::new();

    let parser = Parser::new(md_text);
    for event in parser {
        match event {
            Event::Start(tag) => {
                style_stack.push(tag.clone());
                if let Tag::Item = tag {
                    current_line_spans.push(Span::raw("* "));
                }
            }
            Event::End(tag_end) => {
                style_stack.pop();
                match tag_end {
                    TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => {
                        lines.push(Line::from(current_line_spans.drain(..).collect::<Vec<_>>()));
                    }
                    TagEnd::CodeBlock => {
                        // The text inside code blocks is handled separately
                    }
                    _ => {}
                }
            }
            Event::Text(text) => {
                let in_code_block = style_stack.iter().any(|t| matches!(t, Tag::CodeBlock(_)));

                if in_code_block {
                    // For code blocks, split text by newlines and create separate lines
                    for (i, code_line) in text.split('\n').enumerate() {
                        if i > 0 {
                            lines
                                .push(Line::from(current_line_spans.drain(..).collect::<Vec<_>>()));
                        }
                        current_line_spans.push(Span::styled(
                            code_line.to_string(),
                            Style::default().bg(Color::DarkGray),
                        ));
                    }
                    lines.push(Line::from(current_line_spans.drain(..).collect::<Vec<_>>()));
                } else {
                    let mut style = Style::default();
                    for tag in &style_stack {
                        match tag {
                            Tag::Emphasis => style = style.italic(),
                            Tag::Strong => style = style.bold(),
                            _ => {}
                        }
                    }
                    current_line_spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(text) => {
                current_line_spans.push(Span::styled(
                    text.to_string(),
                    Style::default().bg(Color::DarkGray),
                ));
            }
            Event::HardBreak => {
                lines.push(Line::from(current_line_spans.drain(..).collect::<Vec<_>>()));
            }
            Event::SoftBreak => {
                current_line_spans.push(Span::raw(" "));
            }
            Event::Rule => {
                lines.push(Line::from("≈≈≈≈≈≈≈≈≈≈")); // 将分隔线改为波浪线，强化水元素
            }
            _ => {}
        }
    }
    if !current_line_spans.is_empty() {
        lines.push(Line::from(current_line_spans.drain(..).collect::<Vec<_>>()));
    }

    Text::from(lines)
}


