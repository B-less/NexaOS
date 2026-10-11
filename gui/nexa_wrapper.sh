#!/bin/sh

BOOT_LOG="/disk/logs/boot.log"

log_event() {
    stamp="$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)"

    if [ -z "$stamp" ]; then
        stamp="uptime $(cut -d ' ' -f 1 /proc/uptime 2>/dev/null)"
    fi

    echo "$stamp $*" >> "$BOOT_LOG"
}

run_tool() {
    tool="$1"
    shift

    if command -v "$tool" >/dev/null 2>&1; then
        "$tool" "$@"
        return $?
    fi

    if [ -x /bin/busybox ]; then
        /bin/busybox "$tool" "$@"
        return $?
    fi

    return 127
}

write_udhcpc_script() {
    script="/tmp/nexa_udhcpc.script"

    cat > "$script" <<'EOF'
#!/bin/sh

case "$1" in
    bound|renew)
        ifconfig "$interface" "$ip" netmask "$subnet" broadcast "$broadcast" 2>/dev/null

        route del default 2>/dev/null
        for router_ip in $router
        do
            route add default gw "$router_ip" dev "$interface" 2>/dev/null
            break
        done

        if [ -n "$dns" ]; then
            : > /etc/resolv.conf
            for dns_ip in $dns
            do
                echo "nameserver $dns_ip" >> /etc/resolv.conf
            done
        fi
        ;;

    deconfig)
        ifconfig "$interface" 0.0.0.0 2>/dev/null
        ;;
esac

exit 0
EOF

    chmod +x "$script"
    echo "$script"
}

start_network() {
    iface="${1:-eth0}"
    log="/tmp/nexa_network.log"

    mkdir -p /etc
    run_tool ifconfig lo up >/dev/null 2>&1

    if [ ! -d "/sys/class/net/$iface" ]; then
        echo "Network: no $iface"
        log_event "network: no $iface"
        return 0
    fi

    run_tool ifconfig "$iface" up >/dev/null 2>&1

    if run_tool udhcpc --help >/dev/null 2>&1; then
        script="$(write_udhcpc_script)"

        if run_tool udhcpc -i "$iface" -q -n -t 3 -T 2 -s "$script" > "$log" 2>&1; then
            echo "Network: $iface ready"
            log_event "network: $iface ready"
        else
            echo "Network: $iface DHCP failed"
            log_event "network: $iface DHCP failed"
        fi
    else
        echo "Network: DHCP unavailable"
        log_event "network: DHCP unavailable"
    fi
}

valid_name() {
    case "$1" in
        ''|*[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-]*)
            return 1
            ;;
        *)
            return 0
            ;;
    esac
}

installed_package() {
    grep -q "^$1$" /disk/packages/installed.db 2>/dev/null
}

is_core_package() {
    case " about calculator desktop-gui editor filemgr menu monitor nettest system todo " in
        *" $1 "*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

sync_package_db() {
    tmp="/disk/packages/installed.tmp"

    : > "$tmp" || return 1

    for appdir in /disk/apps/*
    do
        [ -d "$appdir" ] || continue

        package="${appdir##*/}"

        if valid_name "$package" && [ -f "$appdir/run" ]; then
            echo "$package" >> "$tmp"
        fi
    done

    mv "$tmp" /disk/packages/installed.db
}

list_installed_packages() {
    sync_package_db

    if [ -s /disk/packages/installed.db ]; then
        cat /disk/packages/installed.db
    else
        echo "(none)"
    fi
}

list_available_packages() {
    found=0

    for pkgdir in /disk/repo/*
    do
        [ -d "$pkgdir" ] || continue

        package="${pkgdir##*/}"

        if valid_name "$package" && [ -f "$pkgdir/run" ]; then
            echo "$package"
            found=1
        fi
    done

    [ "$found" -eq 1 ] || echo "(none)"
}

package_info() {
    package="$1"

    if [ -z "$package" ]; then
        echo "Usage: pkg info <name>"
    elif ! valid_name "$package"; then
        echo "Invalid package name"
    else
        echo "Package: $package"

        if [ -d "/disk/apps/$package" ]; then
            echo "Installed: yes"
            echo "Install path: /disk/apps/$package"
        else
            echo "Installed: no"
        fi

        if [ -d "/disk/repo/$package" ]; then
            echo "Available: yes"
            echo "Repo path: /disk/repo/$package"
        else
            echo "Available: no"
        fi

        if is_core_package "$package"; then
            echo "Core package: yes"
        else
            echo "Core package: no"
        fi

        if [ -x "/disk/apps/$package/run" ]; then
            echo "Runnable: yes"
        elif [ -f "/disk/apps/$package/run" ]; then
            echo "Runnable: no, run file is not executable"
        else
            echo "Runnable: no"
        fi
    fi
}

verify_packages() {
    issues=0

    sync_package_db

    for appdir in /disk/apps/*
    do
        [ -d "$appdir" ] || continue

        package="${appdir##*/}"

        if ! valid_name "$package"; then
            echo "Invalid app directory: $appdir"
            issues=$((issues + 1))
        elif [ ! -f "$appdir/run" ]; then
            echo "Missing run file: $package"
            issues=$((issues + 1))
        elif [ ! -x "$appdir/run" ]; then
            echo "Run file is not executable: $package"
            issues=$((issues + 1))
        fi
    done

    for pkgdir in /disk/repo/*
    do
        [ -d "$pkgdir" ] || continue

        package="${pkgdir##*/}"

        if ! valid_name "$package"; then
            echo "Invalid repo package: $pkgdir"
            issues=$((issues + 1))
        elif [ ! -f "$pkgdir/run" ]; then
            echo "Repo package missing run file: $package"
            issues=$((issues + 1))
        fi
    done

    if [ "$issues" -eq 0 ]; then
        echo "Package verification passed."
    else
        echo "Package verification found $issues issue(s)."
    fi
}

show_logs() {
    case "$1" in
        ''|boot)
            if [ -s "$BOOT_LOG" ]; then
                cat "$BOOT_LOG"
            else
                echo "No boot logs."
            fi
            ;;

        network)
            if [ -s /tmp/nexa_network.log ]; then
                cat /tmp/nexa_network.log
            else
                echo "No network log for this boot."
            fi
            ;;

        clear)
            : > "$BOOT_LOG"
            echo "Logs cleared."
            ;;

        *)
            echo "Usage: logs [boot|network|clear]"
            ;;
    esac
}

print_diagnostics() {
    echo "Nexa Diagnostics"
    echo "----------------"
    echo "Kernel: $(uname -r)"
    echo "Machine: $(uname -m)"
    echo "Uptime: $(cut -d ' ' -f 1 /proc/uptime 2>/dev/null) seconds"
    echo
    echo "Memory:"
    grep '^MemTotal:\|^MemFree:\|^MemAvailable:' /proc/meminfo 2>/dev/null
    echo
    echo "Disk:"
    df -h /disk 2>/dev/null
    echo
    echo "Network:"
    run_tool ifconfig eth0 2>/dev/null
    echo
    echo "Packages:"
    sync_package_db
    echo "$(wc -l < /disk/packages/installed.db) installed"
}

print_banner() {
    clear
    echo "================================="
    echo "          NEXA OS"
    echo "================================="
    echo
    echo "AI-FIRST OPERATING SYSTEM"
    echo
}

print_help() {
    echo
    echo "Available commands:"
    echo " help"
    echo " about"
    echo " version"
    echo " system"
    echo " disk"
    echo " diag"
    echo " logs [boot|network|clear]"
    echo " desktop"
    echo " gui"
    echo " apps"
    echo " run <app>"
    echo " pkg list"
    echo " pkg available"
    echo " pkg info <name>"
    echo " pkg verify"
    echo " pkg install <name>"
    echo " pkg remove <name>"
    echo " reboot"
    echo " shutdown"
    echo " exit"
    echo
}


cmd="$1"
arg="$2"
arg2="$3"

case "$cmd" in

        help)
            print_help
            ;;

        about)
            echo
            echo "Nexa OS Development Build"
            echo "AI-First Operating System"
            echo
            ;;

        version)
            uname -r
            ;;

        system)
            echo
            echo "Nexa OS Information"
            echo "-------------------"
            echo "Kernel: $(uname -r)"
            echo "Machine: $(uname -m)"
            sync_package_db
            echo "Packages: $(wc -l < /disk/packages/installed.db)"
            echo
            ;;

        disk)
            echo
            echo "Disk Information"
            echo "----------------"
            df -h /disk
            echo
            ;;

        diag)
            echo
            print_diagnostics
            echo
            ;;

        logs)
            echo
            show_logs "$arg"
            echo
            ;;

        desktop|menu)
            if [ -x "/disk/apps/menu/run" ]; then
                "/disk/apps/menu/run"
            else
                echo "Desktop environment not found"
            fi
            ;;

        gui)
            if [ -x "/disk/apps/desktop-gui/run" ]; then
                "/disk/apps/desktop-gui/run"
            else
                echo "Framebuffer GUI not found"
                echo "Boot with ./boot-gui.sh and verify desktop-gui is installed."
            fi
            ;;

        apps)
            echo
            echo "Installed Applications:"
            ls /disk/apps 2>/dev/null
            echo
            ;;

        run)
            if [ -z "$arg" ]; then
                echo "Usage: run <app>"
            elif ! valid_name "$arg"; then
                echo "Invalid application name"
            elif [ -x "/disk/apps/$arg/run" ]; then
                "/disk/apps/$arg/run"
            else
                echo "Application not found"
            fi
            ;;

        pkg)

            case "$arg" in

                list)
                    echo
                    echo "Installed Packages:"
                    list_installed_packages
                    echo
                    ;;

                available)
                    echo
                    echo "Available Packages:"
                    list_available_packages
                    echo
                    ;;

                info)
                    echo
                    package_info "$arg2"
                    echo
                    ;;

                verify)
                    echo
                    verify_packages
                    echo
                    ;;

                install)

                    package="$arg2"

                    if [ -z "$package" ]; then
                        echo "Usage: pkg install <name>"
                    elif ! valid_name "$package"; then
                        echo "Invalid package name"
                    elif [ -d "/disk/apps/$package" ]; then
                        echo "Package already installed"
                    else
                        # First try local repo
                        if [ -d "/disk/repo/$package" ] && [ -f "/disk/repo/$package/run" ]; then
                            tmp="/disk/apps/.install-$package-$$"
                            rm -rf "$tmp"

                            if cp -r "/disk/repo/$package" "$tmp" \
                                && chmod +x "$tmp/run" \
                                && mv "$tmp" "/disk/apps/$package"
                            then
                                sync_package_db
                                echo "Installed $package from local repo"
                            else
                                rm -rf "$tmp"
                                echo "Local install failed"
                            fi
                        else
                            # Try network install
                            echo "Fetching $package from network..."
                            REPO_URL="https://raw.githubusercontent.com/NexaOS/packages/main/apps"
                            FALLBACK_URL="http://10.0.2.2:8000"
                            
                            tmp="/disk/apps/.install-$package-$$"
                            rm -rf "$tmp"
                            mkdir -p "$tmp"
                            
                            if run_tool wget -q -O /tmp/pkg.tar.gz "$REPO_URL/$package.tar.gz" 2>/dev/null || \
                               run_tool wget -q -O /tmp/pkg.tar.gz "$FALLBACK_URL/$package.tar.gz" 2>/dev/null; then
                                
                                if run_tool tar -xzf /tmp/pkg.tar.gz -C "$tmp" 2>/dev/null; then
                                    if [ ! -f "$tmp/run" ] && [ -d "$tmp/$package" ] && [ -f "$tmp/$package/run" ]; then
                                        mv "$tmp/$package"/* "$tmp/" 2>/dev/null
                                        rm -rf "$tmp/$package"
                                    fi

                                    if [ -f "$tmp/run" ]; then
                                        chmod +x "$tmp/run"
                                        mv "$tmp" "/disk/apps/$package"
                                        sync_package_db
                                        echo "Installed $package from network"
                                    else
                                        echo "Network package is missing 'run' file"
                                        rm -rf "$tmp"
                                    fi
                                else
                                    echo "Failed to extract package"
                                    rm -rf "$tmp"
                                fi
                                rm -f /tmp/pkg.tar.gz
                            else
                                echo "Package '$package' not found in local repo or network"
                                rm -rf "$tmp"
                            fi
                        fi
                    fi
                    ;;

                remove)

                    package="$arg2"

                    if [ -z "$package" ]; then
                        echo "Usage: pkg remove <name>"
                    elif ! valid_name "$package"; then
                        echo "Invalid package name"
                    elif is_core_package "$package"; then
                        echo "Cannot remove core package"
                    elif [ ! -d "/disk/apps/$package" ]; then
                        echo "Package is not installed"
                    else

                        if rm -rf "/disk/apps/$package"; then
                            sync_package_db
                            echo "Removed $package"
                        else
                            echo "Remove failed"
                        fi

                    fi
                    ;;

                *)
                    echo
                    echo "Package Manager"
                    echo
                    echo "pkg list"
                    echo "pkg available"
                    echo "pkg info <name>"
                    echo "pkg verify"
                    echo "pkg install <name>"
                    echo "pkg remove <name>"
                    echo
                    ;;
            esac
            ;;

        reboot)
            reboot -f
            ;;

        shutdown)
            poweroff -f
            ;;

        exit)
            echo "Goodbye."
            exit 0
            ;;

        "")
            ;;

        *)
            exec "$@"
            ;;

    esac

