//! Small pieces of editor state that only drive what the toolbar and the
//! window-snapping highlight show.

use crate::geom::{Point, Rect};
use crate::style::Tool;
use crate::toolbar::{Group, Item};

/// Toolbar state that is not a drawing choice.
pub struct ToolbarUi {
    pub hover: Option<Item>,
    /// Group whose flyout is open.
    pub open: Option<Group>,
    /// Last tool used from each tool group, shown on the group's button.
    pub shape_tool: Tool,
    pub draw_tool: Tool,
}

impl Default for ToolbarUi {
    fn default() -> Self {
        Self {
            hover: None,
            open: None,
            shape_tool: Tool::Rect,
            draw_tool: Tool::Pen,
        }
    }
}

impl ToolbarUi {
    /// Remembers `tool` on its group's button.
    pub fn remember(&mut self, tool: Tool) {
        match Group::of_tool(tool) {
            Some(Group::Shapes) => self.shape_tool = tool,
            Some(Group::Draw) => self.draw_tool = tool,
            _ => {}
        }
    }

    /// The tool a tool-group button switches to.
    pub fn tool_for(&self, group: Group) -> Option<Tool> {
        match group {
            Group::Shapes => Some(self.shape_tool),
            Group::Draw => Some(self.draw_tool),
            Group::Color | Group::Size => None,
        }
    }
}

/// Click-to-select-a-window before any area exists.
#[derive(Default)]
pub struct Snapping {
    /// Window frames on this display, front to back (window pixels).
    pub frames: Vec<Rect>,
    /// Window under the cursor.
    pub hover: Option<Rect>,
    /// `hover` at the press that started creating the area.
    pub on_press: Option<Rect>,
    /// Where that press was, to tell a click from a drag.
    pub press_at: Option<Point>,
}

impl Snapping {
    /// Front-most window frame containing `p`.
    pub fn at(&self, p: Point) -> Option<Rect> {
        self.frames.iter().find(|r| r.contains(p)).copied()
    }
}
