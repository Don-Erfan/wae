package com.donerfan.wae

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import com.intellij.platform.lsp.api.LspServerStarter
import com.intellij.platform.lsp.api.LspServerSupportProvider

class WaeLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerStarter) {
        if (file.extension in setOf("js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts")) {
            serverStarter.ensureServerStarted(WaeLspServerDescriptor(project))
        }
    }
}

private class WaeLspServerDescriptor(project: Project) : ProjectWideLspServerDescriptor(project, "WAE") {
    override fun isSupportedFile(file: VirtualFile): Boolean =
        file.extension in setOf("js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts")

    override fun createCommandLine(): GeneralCommandLine =
        GeneralCommandLine(System.getenv("WAE_LSP_PATH") ?: "wae-lsp")
            .withWorkDirectory(project.basePath)
}
