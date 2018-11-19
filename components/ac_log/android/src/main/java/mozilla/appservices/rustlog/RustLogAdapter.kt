/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

package mozilla.appservices.rustlog

import com.sun.jna.Pointer

typealias OnLog = (Int, String?, String) -> Unit;
class RustLogAdapter private constructor(
    // IMPORTANT: This must not be GCed while the adapter is alive!
        @Suppress("Unused")
        private val callbackImpl: RawLogCallbackImpl,
        private val adapter: RawLogAdapter
) {
    companion object {
        @Volatile
        private var instance: RustLogAdapter? = null

        @Volatile
        private var everEnabled: Boolean = false;

        /**
         * true if the log is enabled.
         */
        val isEnabled get() = instance != null

        /**
         * True if the log can be enabled.
         *
         * Note that this isn't the same as `!isEnabled`, as the log
         * cannot be re-enabled after it is disabled.
         */
        val canEnable get() = !everEnabled

        /**
         * Enable the logger and use the provided logging callback.
         *
         * Note that the logger can only be enabled once.
         */
        @Synchronized
        fun enable(onLog: OnLog) {
            val wasEnabled = everEnabled
            everEnabled = true
            if (wasEnabled) {
                throw RuntimeException("Cannot re-enable the log adapter after it has been enabled once")
            }
            val callbackImpl = RawLogCallbackImpl(onLog)
            val err = RustError.ByReference()
            val adapter = LibRustLogAdapter.INSTANCE.ac_log_adapter_create(callbackImpl, err)

            // XXX is there any way for us to half-initialize the logger where the callback could
            // still get called despite an error/null being returned? If so, we need to make
            // callbackImpl isn't GCed here, or very bad things will happen. (Maybe the
            // initialization code should abort on panic...)
            if (err.isFailure()) {
                val msg = err.consumeErrorMessage()
                // No known cases where this is expected (we already ensure things are only
                // initialized once), but something unexpected could panic. As a result, we just
                // throw a RuntimeException.
                throw RuntimeException("Failed to initialize rust logger: $msg")
            }
            instance = RustLogAdapter(callbackImpl, adapter!!)
        }

        /**
         * Helper to enable the logger if it can be enabled. Returns true if
         * the logger was enabled by this call.
         */
        @Synchronized
        fun tryEnable(onLog: OnLog): Boolean {
            if (!canEnable) {
                return false
            }
            enable(onLog)
            return true
        }

        /**
         * Disable the logger, allowing the logging callback to be garbage collected.
         *
         * Note that the logger can only be enabled once.
         */
        @Synchronized
        fun disable() {
            val state = instance ?: return
            LibRustLogAdapter.INSTANCE.ac_log_adapter_destroy(state.adapter)
            // XXX Letting that callback get GCed still makes me extremely uneasy...
            // Maybe we should just null out the callback provided by the user so that
            // it can be GCed (while letting the RawLogCallbackImpl which actually is
            // called by Rust live on).
            instance = null
        }
    }
}

internal class RawLogCallbackImpl(private val onLog: OnLog) : RawLogCallback {
    override fun invoke(level: Int, tag: Pointer?, message: Pointer) {
        // We can't safely throw here!
        try {
            val tagStr = tag?.getString(0, "utf8")
            val msgStr = message.getString(0, "utf8")
            onLog(level, tagStr, msgStr)
        } catch(e: Throwable) {
            try {
                println("Exception when logging: $e")
            } catch (e: Throwable) {
                // :(
            }
        }
    }
}

internal fun Pointer.getAndConsumeRustString(): String {
    try {
        return this.getString(0, "utf8")
    } finally {
        LibRustLogAdapter.INSTANCE.ac_log_adapter_destroy_string(this)
    }
}
