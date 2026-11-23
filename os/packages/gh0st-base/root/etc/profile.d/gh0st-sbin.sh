# Debian leaves the sbin folders off a user's PATH. On a one-user device the
# user wants i2cdetect and friends, which live there.
case ":$PATH:" in
    *:/usr/sbin:*) ;;
    *) PATH="$PATH:/usr/local/sbin:/usr/sbin:/sbin" ;;
esac
