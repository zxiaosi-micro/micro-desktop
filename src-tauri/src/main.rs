//! micro-desktop 运维 PC 入口（Windows GUI 子系统——不弹控制台）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    micro_desktop_lib::run()
}
