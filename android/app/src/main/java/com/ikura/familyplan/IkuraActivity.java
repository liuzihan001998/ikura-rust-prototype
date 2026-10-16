package com.ikura.familyplan;

import android.app.AlertDialog;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

import org.libsdl.app.SDLActivity;

import java.io.BufferedInputStream;
import java.io.BufferedReader;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

public final class IkuraActivity extends SDLActivity {
    private static final String TAG = "Ikura";
    private static final String ASSET_ARCHIVE = "ikura-data.zip";
    private File dataDirectory;
    private View loadingOverlay;
    private boolean assetsReady;
    private boolean resumeRequested;
    private boolean assetsFailed;
    private TouchControls touchControls;

    @Override
    protected boolean onUnhandledMessage(int command, Object param) {
        if (command == 0x8001) {
            if (touchControls == null && !isFinishing() && !isDestroyed()) {
                touchControls = new TouchControls(this, mLayout);
                if (android.os.Build.VERSION.SDK_INT >= 33) {
                    Api33Back.register(this, () -> touchControls.back());
                }
            }
            return true;
        }
        return super.onUnhandledMessage(command, param);
    }

    @Override
    public void onBackPressed() {
        if (touchControls != null) touchControls.back();
        else super.onBackPressed();
    }

    @Override
    public boolean dispatchKeyEvent(android.view.KeyEvent event) {
        if (touchControls != null && event.getKeyCode() == android.view.KeyEvent.KEYCODE_BACK) {
            if (event.getAction() == android.view.KeyEvent.ACTION_UP && !event.isCanceled()) {
                touchControls.back();
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @android.annotation.TargetApi(33)
    private static final class Api33Back {
        static void register(IkuraActivity activity, Runnable action) {
            activity.getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, action::run);
        }
    }

    @Override
    protected void onPause() {
        if (touchControls != null) touchControls.releaseHeldKeys();
        super.onPause();
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (mBrokenLibraries) {
            return;
        }
        setWindowStyle(true);
        dataDirectory = new File(getFilesDir(), "ikura/game_data");
        File readyMarker = new File(dataDirectory, ".ready");
        if (hasCurrentAssets(readyMarker)) {
            assetsReady = true;
            return;
        }

        showLoadingOverlay();
        Thread extraction = new Thread(() -> {
            try {
                extractAssets(readyMarker);
                runOnUiThread(() -> {
                    if (isFinishing() || isDestroyed()) {
                        return;
                    }
                    assetsReady = true;
                    Log.i(TAG, "Bundled game resources are ready");
                    removeLoadingOverlay();
                    startGameIfReady();
                });
            } catch (Exception error) {
                Log.e(TAG, "Could not prepare bundled game data", error);
                runOnUiThread(() -> {
                    if (!isFinishing() && !isDestroyed()) {
                        showAssetError(error);
                    }
                });
            }
        }, "IkuraAssetSetup");
        extraction.start();
    }

    private void showLoadingOverlay() {
        LinearLayout panel = new LinearLayout(this);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setGravity(Gravity.CENTER);
        panel.setPadding(48, 32, 48, 32);
        panel.setBackgroundColor(0xFF101014);

        ProgressBar spinner = new ProgressBar(this);
        panel.addView(spinner, new LinearLayout.LayoutParams(72, 72));

        TextView message = new TextView(this);
        message.setText("首次启动，正在准备游戏资源，请稍候…");
        message.setTextColor(0xFFFFFFFF);
        message.setTextSize(18);
        message.setGravity(Gravity.CENTER);
        LinearLayout.LayoutParams textParams = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        textParams.topMargin = 28;
        panel.addView(message, textParams);

        FrameLayout overlay = new FrameLayout(this);
        overlay.setBackgroundColor(0xFF101014);
        overlay.addView(panel, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
                Gravity.CENTER));
        mLayout.addView(overlay, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));
        loadingOverlay = overlay;
    }

    private void removeLoadingOverlay() {
        if (loadingOverlay != null) {
            mLayout.removeView(loadingOverlay);
            loadingOverlay = null;
        }
    }

    private void extractAssets(File readyMarker) throws Exception {
        File appRoot = dataDirectory.getParentFile();
        if (appRoot == null) {
            throw new IllegalStateException("Invalid app data path");
        }
        if (!appRoot.exists() && !appRoot.mkdirs()) {
            throw new IllegalStateException("Could not create " + appRoot);
        }
        if (dataDirectory.exists()) {
            deleteTree(dataDirectory);
        }

        try (InputStream raw = getAssets().open(ASSET_ARCHIVE);
             ZipInputStream zip = new ZipInputStream(new BufferedInputStream(raw))) {
            byte[] buffer = new byte[64 * 1024];
            ZipEntry entry;
            String rootPath = appRoot.getCanonicalPath() + File.separator;
            while ((entry = zip.getNextEntry()) != null) {
                if (entry.getName().contains("..")) {
                    throw new IllegalStateException("Unsafe asset path: " + entry.getName());
                }
                File output = new File(appRoot, entry.getName());
                if (!output.getCanonicalPath().startsWith(rootPath)) {
                    throw new IllegalStateException("Unsafe asset path: " + entry.getName());
                }
                if (entry.isDirectory()) {
                    if (!output.mkdirs() && !output.isDirectory()) {
                        throw new IllegalStateException("Could not create " + output);
                    }
                } else {
                    File parent = output.getParentFile();
                    if (parent != null && !parent.mkdirs() && !parent.isDirectory()) {
                        throw new IllegalStateException("Could not create " + parent);
                    }
                    try (FileOutputStream file = new FileOutputStream(output)) {
                        int count;
                        while ((count = zip.read(buffer)) != -1) {
                            file.write(buffer, 0, count);
                        }
                        file.getFD().sync();
                    }
                }
                zip.closeEntry();
            }
        }

        if (!new File(dataDirectory, "scripts.json").isFile()) {
            throw new IllegalStateException("The bundled archive has no game_data/scripts.json");
        }
        try (FileOutputStream marker = new FileOutputStream(readyMarker)) {
            marker.write(Integer.toString(BuildConfig.VERSION_CODE).getBytes(java.nio.charset.StandardCharsets.UTF_8));
            marker.getFD().sync();
        }
    }

    private boolean hasCurrentAssets(File readyMarker) {
        if (!readyMarker.isFile() || !new File(dataDirectory, "scripts.json").isFile()) {
            return false;
        }
        try (BufferedReader reader = new BufferedReader(new InputStreamReader(
                new FileInputStream(readyMarker), java.nio.charset.StandardCharsets.UTF_8))) {
            String version = reader.readLine();
            return Integer.toString(BuildConfig.VERSION_CODE).equals(version);
        } catch (Exception ignored) {
            return false;
        }
    }

    private void deleteTree(File file) {
        File[] children = file.listFiles();
        if (children != null) {
            for (File child : children) {
                deleteTree(child);
            }
        }
        file.delete();
    }

    private void showAssetError(Exception error) {
        assetsFailed = true;
        new AlertDialog.Builder(this)
                .setTitle("游戏资源准备失败")
                .setMessage(error.getMessage() + "\n请重新安装应用后再试。")
                .setCancelable(false)
                .setPositiveButton("退出", (dialog, which) -> finish())
                .show();
    }

    private void startGameIfReady() {
        if (resumeRequested && assetsReady && !assetsFailed) {
            super.resumeNativeThread();
        }
    }

    @Override
    protected void resumeNativeThread() {
        resumeRequested = true;
        if (assetsReady && !assetsFailed) {
            super.resumeNativeThread();
        }
    }

    @Override
    protected void pauseNativeThread() {
        resumeRequested = false;
        super.pauseNativeThread();
    }

    @Override
    protected void onDestroy() {
        resumeRequested = false;
        if (touchControls != null) touchControls.dispose();
        super.onDestroy();
    }

    @Override
    protected String[] getLibraries() {
        return new String[] { "SDL2", "SDL2_mixer", "main" };
    }

    @Override
    protected String[] getArguments() {
        File scriptFile = new File(dataDirectory, "scripts.json");
        return new String[] {
                dataDirectory.getAbsolutePath(),
                scriptFile.getAbsolutePath(),
                "--lang=zh-CN"
        };
    }
}
