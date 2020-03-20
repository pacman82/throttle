def is_recoverable(status_code: int) -> bool:
    """
    True if the passed status_code hints at a recoverable server error. I.e. The same request might
    be successful at a later point in time.
    """
    if status_code % 100 == 4:
        #Request Timeout, Connection Closed Without Response, Client Closed Request
        if status_code in [408, 444, 499]:
            return True
        else:
            # If the error is on client side we shouldn't just repeat it, for the most part.
            return False
    elif status_code % 100 == 5:
        # Not implemented, HTTP Version not supported, Variant also negoiates, Insufficient Storage,
        # Loop Detected, Not Extended, Network Authentication Required
        if status_code in [501, 505, 506, 507, 508, 510,511]:
            return False
        else:
            # In general server errors may be fixed later
            return True
    raise ValueError(f"Invalid http status code {status_code}")

