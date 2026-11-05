# Shared host/hermetic Prettier leaves. Call with --working-directory <checkout>.
prettier-markdown mode="verify":
    prettier {{if mode == "fix" { "-w" } else { "--check" }}} '**/*.md'

prettier-end2end mode="verify":
    prettier {{if mode == "fix" { "-w" } else { "--check" }}} end2end

prettier-css mode="verify":
    prettier {{if mode == "fix" { "-w" } else { "--check" }}} '**/*.css'
