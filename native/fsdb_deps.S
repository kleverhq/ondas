/* Strong references keep SDK dependencies under lld --as-needed. The SDK's
 * libnffr does not declare libnsys/zlib as DT_NEEDED; no symbol data is accessed. */
.section .data.rel.ro,"aw",@progbits
.balign 8
.global ondas_fsdb_dependencies
.hidden ondas_fsdb_dependencies
.type ondas_fsdb_dependencies,@object
ondas_fsdb_dependencies:
.quad prodCnlIdent
.quad inflate
.size ondas_fsdb_dependencies, .-ondas_fsdb_dependencies
.section .note.GNU-stack,"",@progbits
