# OPS-11/OPS-12: an immutable archive must not retain an older download pin
# or the moving checkout's publication status. Only private staging is edited.
BEGIN {
    base = "https://github.com/synveda/synveda/blob/" source "/"
}
(main && /^<!-- installation-version:/) || (examples && /^<!-- chart-package-source-status:start -->$/) {
    status = 1
    status_start++
    print "**Chart archive " version ".** This guide belongs to source `" source "`."
    print "Use only an approved release with its publisher attestation, checksums and"
    print "matching image overlay. A privately packaged candidate is not publisher proof."
    print ""
    next
}
(main || examples) && status && /^<!-- chart-package-source-status:end -->$/ {
    status = 0
    status_end++
    next
}
(main || examples) && status { next }
main && /^<!-- chart-package-source-identity:start -->$/ {
    identity = 1
    identity_start++
    print "The source commit below identifies this archive. Confirm its version and"
    print "source against the independently reviewed release record. Never copy an"
    print "expected commit from an unverified downloaded inventory. Install GitHub CLI"
    print "independently of these assets."
    next
}
main && identity && /^<!-- chart-package-source-identity:end -->$/ {
    identity = 0
    identity_end++
    next
}
main && identity { next }
main && /^  RELEASE_VERSION=/ {
    print "  RELEASE_VERSION=" version
    version_pin++
    next
}
main && /^  SOURCE_SHA=/ {
    print "  SOURCE_SHA=" source
    source_pin++
    next
}
{
    # Only Markdown links outside the archive are rebound. Local chart guides
    # remain usable beside the extracted tools, without a source checkout.
    if (examples) {
        gsub(/\]\(\.\.\/\.\.\/\.\.\/\.\.\//, "](" base)
        gsub(/\]\(\.\.\/\.\.\/\.\.\//, "](" base "deploy/")
    } else {
        gsub(/\]\(\.\.\/\.\.\/\.\.\//, "](" base)
        gsub(/\]\(\.\.\/\.\.\//, "](" base "deploy/")
    }
    gsub(/https:\/\/github.com\/synveda\/synveda\/blob\/main\//, base)
    print
}
END {
    if (examples && (status || status_start != 1 || status_end != 1)) {
        print "package-chart: guide identity boundaries changed; restore or review the matching checkout" > "/dev/stderr"
        exit 66
    }
    if (main && (status || identity || status_start != 1 || status_end != 1 ||
        identity_start != 1 || identity_end != 1 || version_pin != 1 || source_pin != 1)) {
        print "package-chart: guide identity boundaries changed; restore or review the matching checkout" > "/dev/stderr"
        exit 66
    }
}
