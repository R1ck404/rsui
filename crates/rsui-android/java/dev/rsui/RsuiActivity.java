package dev.rsui;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.DatePickerDialog;
import android.app.Dialog;
import android.app.TimePickerDialog;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.DialogInterface;
import android.content.Intent;
import android.content.res.Configuration;
import android.graphics.Color;
import android.os.Bundle;
import android.text.InputType;
import android.text.format.DateFormat;
import android.view.ContextThemeWrapper;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.DatePicker;
import android.widget.TimePicker;
import android.widget.Toast;

import java.util.Calendar;
import java.util.Locale;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The activity an rsui app runs in. It hands its window's surface to rsui, which draws it, and keeps one view of its own,
 * which is never drawn, for what comes in: touches, keys, and the soft keyboard's text through an InputConnection, which
 * Android's own NativeActivity has no way to give. Everything is passed to the native library as it comes, and the app's
 * own thread there does the rest.
 */
public class RsuiActivity extends Activity implements SurfaceHolder.Callback2 {
    static {
        System.loadLibrary("rsui_app");
    }

    private RsuiView view;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().takeSurface(this);
        view = new RsuiView(this);
        setContentView(view);
        view.requestFocus();
        nativeCreate(this, getAssets(), getFilesDir().getPath(), getCacheDir().getPath(),
                getResources().getDisplayMetrics().density, isNight(getResources().getConfiguration()));
    }

    private static boolean isNight(Configuration configuration) {
        return (configuration.uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        nativeConfiguration(getResources().getDisplayMetrics().density, isNight(configuration));
    }

    @Override
    protected void onResume() {
        super.onResume();
        nativeShown(true);
    }

    @Override
    protected void onPause() {
        nativeShown(false);
        super.onPause();
    }

    @Override
    public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        nativeFocus(focused);
    }

    @Override
    public void onLowMemory() {
        super.onLowMemory();
        nativeLowMemory();
    }

    @Override
    protected void onDestroy() {
        nativeDestroy();
        super.onDestroy();
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        nativeSurface(holder.getSurface());
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        nativeSurface(holder.getSurface());
    }

    @Override
    public void surfaceRedrawNeeded(SurfaceHolder holder) {
        nativeRedraw();
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurface(null);
    }

    /** Called by the app's thread when a field that takes text is focused, or isn't any more. */
    public void showKeyboard(final boolean show) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                InputMethodManager keyboard = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (show) {
                    view.requestFocus();
                    keyboard.restartInput(view);
                    keyboard.showSoftInput(view, 0);
                } else {
                    keyboard.hideSoftInputFromWindow(view.getWindowToken(), 0);
                }
            }
        });
    }

    /**
     * Called by the app's thread to show one of the system's popups, which is answered through nativePopupAnswered with
     * the same id: "true" or "false" for a confirm, a place in the list for a sheet, "2026-11-21" for a day, "14:05" for a
     * time, anything for an alert or share, and null for none; or "!" and why it couldn't be shown.
     */
    public void popup(final int id, final String kind, final String json) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Answer answer = new Answer(id);
                try {
                    show(answer, kind, new JSONObject(json));
                } catch (JSONException | RuntimeException failed) {
                    answer.say("!" + failed);
                }
            }
        });
    }

    /** Called by the app's thread to show a toast. */
    public void toast(final String text, final boolean lasting) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Toast.makeText(RsuiActivity.this, text, lasting ? Toast.LENGTH_LONG : Toast.LENGTH_SHORT).show();
            }
        });
    }

    /** A popup's answer, which is only said once, whichever way it's answered. */
    static class Answer {
        private final int id;
        private boolean said;

        Answer(int id) {
            this.id = id;
        }

        void say(String answer) {
            if (!said) {
                said = true;
                nativePopupAnswered(id, answer);
            }
        }

        /** Says this once the dialog's gone, however it went, unless something's been said already. */
        void otherwise(Dialog dialog, final String answer) {
            dialog.setOnDismissListener(new DialogInterface.OnDismissListener() {
                @Override
                public void onDismiss(DialogInterface gone) {
                    say(answer);
                }
            });
        }
    }

    /** The activity with the system's own light or dark look, as it is now, for the popups. */
    private Context themed() {
        int theme = isNight(getResources().getConfiguration())
                ? android.R.style.Theme_DeviceDefault
                : android.R.style.Theme_DeviceDefault_Light;
        return new ContextThemeWrapper(this, theme);
    }

    private static int[] day(String text) {
        String[] parts = text.split("-");
        return new int[] { Integer.parseInt(parts[0]), Integer.parseInt(parts[1]), Integer.parseInt(parts[2]) };
    }

    private static long dayMillis(String text) {
        int[] day = day(text);
        Calendar calendar = Calendar.getInstance();
        calendar.clear();
        calendar.set(day[0], day[1] - 1, day[2]);
        return calendar.getTimeInMillis();
    }

    private static AlertDialog.Builder titled(Context context, String title, String message) {
        AlertDialog.Builder builder = new AlertDialog.Builder(context);
        if (!title.isEmpty()) {
            builder.setTitle(title);
        }
        if (!message.isEmpty()) {
            builder.setMessage(message);
        }
        return builder;
    }

    private void show(final Answer answer, String kind, JSONObject asked) throws JSONException {
        Context context = themed();
        String title = asked.optString("title", "");
        String message = asked.optString("message", "");
        switch (kind) {
            case "confirm": {
                AlertDialog.Builder builder = titled(context, title, message);
                builder.setPositiveButton(asked.optString("confirm", getString(android.R.string.ok)), new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        answer.say("true");
                    }
                });
                builder.setNegativeButton(asked.optString("cancel", getString(android.R.string.cancel)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, "false");
                dialog.show();
                if (asked.optBoolean("destructive", false)) {
                    // Material's color for what can't be undone.
                    Button confirm = dialog.getButton(DialogInterface.BUTTON_POSITIVE);
                    if (confirm != null) {
                        boolean night = isNight(getResources().getConfiguration());
                        confirm.setTextColor(night ? Color.rgb(0xF2, 0xB8, 0xB5) : Color.rgb(0xB3, 0x26, 0x1E));
                    }
                }
                break;
            }
            case "alert": {
                AlertDialog.Builder builder = titled(context, title, message);
                builder.setPositiveButton(asked.optString("ok", getString(android.R.string.ok)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, "");
                dialog.show();
                break;
            }
            case "sheet": {
                JSONArray given = asked.getJSONArray("options");
                String[] options = new String[given.length()];
                for (int at = 0; at < options.length; at++) {
                    options[at] = given.getString(at);
                }
                AlertDialog.Builder builder = titled(context, title, "");
                builder.setItems(options, new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        answer.say(Integer.toString(which));
                    }
                });
                builder.setNegativeButton(asked.optString("cancel", getString(android.R.string.cancel)), null);
                AlertDialog dialog = builder.create();
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "date": {
                Calendar now = Calendar.getInstance();
                int[] start = asked.has("value") ? day(asked.getString("value"))
                        : new int[] { now.get(Calendar.YEAR), now.get(Calendar.MONTH) + 1, now.get(Calendar.DAY_OF_MONTH) };
                DatePickerDialog dialog = new DatePickerDialog(context, new DatePickerDialog.OnDateSetListener() {
                    @Override
                    public void onDateSet(DatePicker picker, int year, int month, int day) {
                        answer.say(String.format(Locale.ROOT, "%04d-%02d-%02d", year, month + 1, day));
                    }
                }, start[0], start[1] - 1, start[2]);
                if (asked.has("min")) {
                    dialog.getDatePicker().setMinDate(dayMillis(asked.getString("min")));
                }
                if (asked.has("max")) {
                    dialog.getDatePicker().setMaxDate(dayMillis(asked.getString("max")));
                }
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "time": {
                Calendar now = Calendar.getInstance();
                int hours = now.get(Calendar.HOUR_OF_DAY);
                int minutes = now.get(Calendar.MINUTE);
                if (asked.has("value")) {
                    String[] parts = asked.getString("value").split(":");
                    hours = Integer.parseInt(parts[0]);
                    minutes = Integer.parseInt(parts[1]);
                }
                TimePickerDialog dialog = new TimePickerDialog(context, new TimePickerDialog.OnTimeSetListener() {
                    @Override
                    public void onTimeSet(TimePicker picker, int hours, int minutes) {
                        answer.say(String.format(Locale.ROOT, "%02d:%02d", hours, minutes));
                    }
                }, hours, minutes, DateFormat.is24HourFormat(this));
                answer.otherwise(dialog, null);
                dialog.show();
                break;
            }
            case "share": {
                String text = asked.optString("text", "");
                String url = asked.optString("url", "");
                String said = text.isEmpty() ? url : url.isEmpty() ? text : text + "\n" + url;
                Intent send = new Intent(Intent.ACTION_SEND);
                send.setType("text/plain");
                send.putExtra(Intent.EXTRA_TEXT, said);
                if (!title.isEmpty()) {
                    send.putExtra(Intent.EXTRA_SUBJECT, title);
                    send.putExtra(Intent.EXTRA_TITLE, title);
                }
                try {
                    startActivity(Intent.createChooser(send, title.isEmpty() ? null : title));
                    answer.say("");
                } catch (ActivityNotFoundException failed) {
                    answer.say("!nothing on this phone can share that");
                }
                break;
            }
            default:
                answer.say("!there's no popup called " + kind);
        }
    }

    /** The view that's never drawn, which input comes to. */
    static class RsuiView extends View {
        RsuiView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
            int action = event.getActionMasked();
            // A move is of every finger that's down; anything else is of the one it says.
            if (action == MotionEvent.ACTION_MOVE) {
                for (int at = 0; at < event.getPointerCount(); at++) {
                    nativeTouch(action, event.getPointerId(at), event.getX(at), event.getY(at));
                }
            } else {
                int index = event.getActionIndex();
                nativeTouch(action, event.getPointerId(index), event.getX(index), event.getY(index));
            }
            return true;
        }

        @Override
        public boolean onKeyDown(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK) {
                return super.onKeyDown(code, event);
            }
            nativeKey(code, event.getMetaState(), true, event.getRepeatCount(), event.getUnicodeChar());
            return true;
        }

        @Override
        public boolean onKeyUp(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK) {
                return super.onKeyUp(code, event);
            }
            nativeKey(code, event.getMetaState(), false, 0, 0);
            return true;
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_AUTO_CORRECT;
            info.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
            return new RsuiInput(this);
        }
    }

    /** What the soft keyboard says, as rsui's own composing text and typing. */
    static class RsuiInput extends BaseInputConnection {
        RsuiInput(View view) {
            super(view, false);
        }

        @Override
        public boolean setComposingText(CharSequence text, int cursor) {
            nativeCompose(text.toString());
            return true;
        }

        @Override
        public boolean finishComposingText() {
            nativeFinishComposing();
            return true;
        }

        @Override
        public boolean commitText(CharSequence text, int cursor) {
            nativeCommit(text.toString());
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int before, int after) {
            nativeDelete(before, after);
            return true;
        }

        @Override
        public boolean sendKeyEvent(KeyEvent event) {
            boolean down = event.getAction() == KeyEvent.ACTION_DOWN;
            nativeKey(event.getKeyCode(), event.getMetaState(), down, event.getRepeatCount(), down ? event.getUnicodeChar() : 0);
            return true;
        }
    }

    static native void nativeCreate(RsuiActivity activity, Object assets, String files, String cache, float density, boolean night);
    static native void nativeConfiguration(float density, boolean night);
    static native void nativeShown(boolean shown);
    static native void nativeFocus(boolean focused);
    static native void nativeLowMemory();
    static native void nativeDestroy();
    static native void nativeSurface(Object surface);
    static native void nativeRedraw();
    static native void nativeTouch(int action, int pointer, float x, float y);
    static native void nativeKey(int code, int meta, boolean down, int repeat, int unicode);
    static native void nativeCompose(String text);
    static native void nativeFinishComposing();
    static native void nativeCommit(String text);
    static native void nativeDelete(int before, int after);
    static native void nativePopupAnswered(int id, String answer);
}
