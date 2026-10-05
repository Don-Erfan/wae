package com.donerfan.wae

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter
import java.io.File

private val SUPPORTED_EXTENSIONS = setOf("js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts")

class WaeLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerStarter) {
        if (file.extension in SUPPORTED_EXTENSIONS) {
            serverStarter.ensureServerStarted(WaeLspServerDescriptor(project))
        }
    }
}

private class WaeLspServerDescriptor(project: Project) : ProjectWideLspServerDescriptor(project, "WAE") {
    override fun isSupportedFile(file: VirtualFile): Boolean = file.extension in SUPPORTED_EXTENSIONS

    // All diagnostics, quick fixes (explain rule, dependency path, documented suppression) and
    // hover text come from the shared wae-lsp server; this plugin only locates and starts it.
    override fun createCommandLine(): GeneralCommandLine =
        GeneralCommandLine(serverExecutable()).withWorkDirectory(project.basePath)

    /** WAE_LSP_PATH, then the project's npm-installed `@don-erfan/wae` launcher, then PATH. */
    private fun serverExecutable(): String {
        System.getenv("WAE_LSP_PATH")?.takeIf { it.isNotBlank() }?.let { return it }
        val launcher = if (SystemInfo.isWindows) "wae-lsp.cmd" else "wae-lsp"
        val local = project.basePath?.let { File(it, "node_modules/.bin/$launcher") }
        return if (local != null && local.isFile) local.path else "wae-lsp"
    }
}
