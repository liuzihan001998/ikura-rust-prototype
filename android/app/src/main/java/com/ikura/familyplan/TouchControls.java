package com.ikura.familyplan;

import android.app.AlertDialog;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.GridLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.libsdl.app.SDLActivity;

/** Native touch controls forward the same key events as a physical keyboard. */
final class TouchControls {
    private final IkuraActivity activity;
    private final ViewGroup parent;
    private final FrameLayout root;
    private final Button toggle;
    private View modal;
    private View keyboard;
    private boolean skipping;

    TouchControls(IkuraActivity activity, ViewGroup parent) {
        this.activity = activity;
        this.parent = parent;
        root = new FrameLayout(activity);
        parent.addView(root, new ViewGroup.LayoutParams(-1, -1));
        toggle = button("菜单", () -> {
            if (skipping) {
                setSkip(false);
            } else if (modal != null) {
                hideMenu();
            } else {
                showMenu();
            }
        });
        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(dp(76), dp(44), Gravity.TOP | Gravity.END);
        params.setMargins(dp(8), dp(8), dp(8), 0);
        root.addView(toggle, params);
    }

    private int dp(int value) {
        return Math.round(value * activity.getResources().getDisplayMetrics().density);
    }

    private Button button(String label, Runnable action) {
        Button button = new Button(activity);
        button.setText(label);
        button.setTextSize(12);
        button.setTextColor(0xFFF1F4FA);
        button.setBackgroundTintList(android.content.res.ColorStateList.valueOf(0xEE26364B));
        button.setPadding(dp(4), 0, dp(4), 0);
        button.setMinimumWidth(0);
        button.setMinimumHeight(0);
        button.setFocusable(false);
        button.setAllCaps(false);
        button.setOnClickListener(view -> action.run());
        return button;
    }

    private static void key(int code) {
        SDLActivity.onNativeKeyDown(code);
        SDLActivity.onNativeKeyUp(code);
    }

    private void action(int code) {
        hideMenu();
        key(code);
    }

    private void setSkip(boolean active) {
        if (skipping == active) return;
        skipping = active;
        if (active) SDLActivity.onNativeKeyDown(KeyEvent.KEYCODE_CTRL_LEFT);
        else SDLActivity.onNativeKeyUp(KeyEvent.KEYCODE_CTRL_LEFT);
        toggle.setText(active ? "停止快进" : "菜单");
    }

    private void cell(GridLayout grid, String label, Runnable action, int width) {
        GridLayout.LayoutParams params = new GridLayout.LayoutParams();
        params.width = dp(width);
        params.height = dp(44);
        params.setMargins(dp(2), dp(2), dp(2), dp(2));
        grid.addView(button(label, action), params);
    }

    private void showMenu() {
        hideKeyboard();
        key(KeyEvent.KEYCODE_F11); // Pause game timers while this modal is open.
        FrameLayout backdrop = new FrameLayout(activity);
        backdrop.setBackgroundColor(0x99000000);
        backdrop.setOnClickListener(view -> hideMenu());
        LinearLayout panel = new LinearLayout(activity);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(dp(8), dp(8), dp(8), dp(8));
        panel.setBackgroundColor(0xFF152033);
        panel.setClickable(true);
        TextView heading = new TextView(activity);
        heading.setText("游戏菜单");
        heading.setTextSize(18);
        heading.setTextColor(0xFFFFFFFF);
        heading.setPadding(dp(4), 0, 0, dp(4));
        panel.addView(heading);
        GridLayout grid = new GridLayout(activity);
        grid.setColumnCount(3);
        cell(grid, "存档", () -> action(KeyEvent.KEYCODE_F6), 100);
        cell(grid, "读档", () -> action(KeyEvent.KEYCODE_F8), 100);
        cell(grid, "快速存档", () -> action(KeyEvent.KEYCODE_F5), 100);
        cell(grid, "快速读档", () -> action(KeyEvent.KEYCODE_F9), 100);
        cell(grid, "切换自动阅读", () -> action(KeyEvent.KEYCODE_A), 100);
        cell(grid, "开启快进", () -> { hideMenu(); setSkip(true); }, 100);
        cell(grid, "历史记录", () -> action(KeyEvent.KEYCODE_B), 100);
        cell(grid, "隐藏/显示文字", () -> action(KeyEvent.KEYCODE_H), 100);
        cell(grid, "设置", () -> action(KeyEvent.KEYCODE_F10), 100);
        cell(grid, "按键面板", this::showKeyboard, 100);
        cell(grid, "继续游戏", this::hideMenu, 100);
        cell(grid, "退出游戏", () -> new AlertDialog.Builder(activity)
                .setTitle("退出游戏")
                .setMessage("未保存的进度会丢失，确定退出吗？")
                .setNegativeButton("取消", null)
                .setPositiveButton("退出", (dialog, which) -> {
                    hideMenu();
                    activity.finish();
                }).show(), 100);
        panel.addView(grid);
        backdrop.addView(panel, new FrameLayout.LayoutParams(-2, -2, Gravity.CENTER));
        modal = backdrop;
        root.addView(backdrop, new FrameLayout.LayoutParams(-1, -1));
        toggle.bringToFront();
    }

    private void hideMenu() {
        if (modal == null) return;
        root.removeView(modal);
        modal = null;
        key(KeyEvent.KEYCODE_F11);
    }

    private void showKeyboard() {
        hideMenu();
        LinearLayout panel = new LinearLayout(activity);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setBackgroundColor(0xEE152033);
        TextView header = new TextView(activity);
        header.setText("按键面板 · 拖动这里移动");
        header.setTextSize(13);
        header.setTextColor(0xFFFFFFFF);
        header.setPadding(dp(8), dp(8), dp(8), dp(8));
        panel.addView(header);
        header.setOnTouchListener(new View.OnTouchListener() {
            private float startX, startY;
            private int left, top;
            @Override public boolean onTouch(View view, MotionEvent event) {
                FrameLayout.LayoutParams params = (FrameLayout.LayoutParams) panel.getLayoutParams();
                if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                    startX = event.getRawX(); startY = event.getRawY();
                    left = params.leftMargin; top = params.topMargin;
                } else if (event.getActionMasked() == MotionEvent.ACTION_MOVE) {
                    params.leftMargin = Math.max(0, Math.min(root.getWidth() - panel.getWidth(), left + (int)(event.getRawX() - startX)));
                    params.topMargin = Math.max(0, Math.min(root.getHeight() - panel.getHeight(), top + (int)(event.getRawY() - startY)));
                    panel.setLayoutParams(params);
                }
                return true;
            }
        });
        GridLayout grid = new GridLayout(activity);
        grid.setColumnCount(5);
        String[] labels = {"↑", "↓", "←", "→", "确认", "空格", "取消/Esc", "切换/Tab", "删除", "上一页", "下一页", "最前", "最后", "自动档", "快速档", "是/Y", "否/N"};
        int[] keys = {KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT, KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_SPACE, KeyEvent.KEYCODE_ESCAPE, KeyEvent.KEYCODE_TAB, KeyEvent.KEYCODE_FORWARD_DEL, KeyEvent.KEYCODE_PAGE_UP, KeyEvent.KEYCODE_PAGE_DOWN, KeyEvent.KEYCODE_MOVE_HOME, KeyEvent.KEYCODE_MOVE_END, KeyEvent.KEYCODE_F7, KeyEvent.KEYCODE_Q, KeyEvent.KEYCODE_Y, KeyEvent.KEYCODE_N};
        for (int i = 0; i < keys.length; i++) {
            final int code = keys[i];
            cell(grid, labels[i], () -> key(code), 60);
        }
        cell(grid, "Ctrl快进", () -> setSkip(!skipping), 60);
        cell(grid, "菜单", this::showMenu, 60);
        cell(grid, "收起", this::hideKeyboard, 60);
        panel.addView(grid);
        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(-2, -2, Gravity.TOP | Gravity.START);
        params.leftMargin = dp(8); params.topMargin = dp(56);
        keyboard = panel;
        root.addView(panel, params);
    }

    private void hideKeyboard() {
        if (keyboard == null) return;
        root.removeView(keyboard);
        keyboard = null;
    }

    void back() {
        if (modal != null) hideMenu();
        else if (keyboard != null) hideKeyboard();
        else if (skipping) setSkip(false);
        else showMenu();
    }

    void releaseHeldKeys() { setSkip(false); }

    void dispose() {
        setSkip(false);
        hideMenu();
        parent.removeView(root);
    }
}
