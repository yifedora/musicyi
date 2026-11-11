use ratatui::style::Color;

/// 极简配色：强调色为青色，品牌色为红橙 #F86424，其余全部使用终端默认前景。
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    /// 红橙 #F86424：首页 MUSICYI 字标；列表当前播放行；播放条歌名。
    pub bender: Color,
}

pub const THEME: Theme = Theme {
    accent: Color::Cyan,
    bender: Color::Rgb(248, 100, 36),
};
