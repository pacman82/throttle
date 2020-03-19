def is_recoverable(status_code: int) -> bool:
    """
    True if the passed status_code hints at a recoverable server error. I.e. The same request might
    be successful at a later point in time.
    """
    if status_code < 400:
        # Not an error. Let's classify it as recoverable using our definition provided above.
        return True
    elif status_code >= 400 and status_code <= 407:
        return False
    elif status_code == 408:
        # Request Timeout
        return True
    elif status_code >= 409 and status_code <= 431:
        return False
    elif status_code == 444:
        # Connection Closed Without Response
        return True
    elif status_code == 451:
        # Unavailable For Legal Reasons
        return False
    elif status_code == 499:
        # Client Closed Request
        return True
    elif status_code == 500:
        # Internal Server Error
        return True
    elif status_code >= 501 and status_code <= 502:
        return False
    elif status_code >= 503 and status_code <= 504:
        return True
    elif status_code >= 505 and status_code <= 511:
        return False
    elif status_code == 599:
        # Network Connection timeout
        return True
    else:
        raise ValueError(f"Invalid http status code {status_code}")
