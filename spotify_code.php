<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Spotify Code</title>
    <style>
        body {
            background-color: #1a1a1a;
            color: #fff;
            margin: 0;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            font-family: 'Arial', sans-serif;
        }
        .container {
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100%;
            width: 100%;
        }
        input[type="text"] {
            font-size: 48px;
            padding: 30px;
            text-align: center;
            width: 80%;
            border-radius: 10px;
            border: none;
            background-color: #222;
            color: #fff;
            transition: background-color 0.3s;
        }
        input[type="text"]:hover {
            background-color: #444;
        }
        input[type="text"]:focus {
            outline: none;
        }
        .notification {
            position: fixed;
            top: 20px;
            left: 50%;
            transform: translateX(-50%);
            padding: 15px 30px;
            background-color: #777;
            color: #fff;
            border-radius: 10px;
            font-size: 24px;
            display: none;
            box-shadow: 0 4px 8px rgba(0, 0, 0, 0.2);
        }
    </style>
</head>
<body>
    <div class="container">
        <?php if (isset($_GET['code'])) { ?>
            <input type="text" id="code" value="<?php echo htmlspecialchars($_GET['code'], ENT_QUOTES, 'UTF-8'); ?>" readonly />
        <?php } ?>
        <div class="notification">Code copied to clipboard!</div>
    </div>
    <script>
        function copyCodeToClipboard() {
            var codeInput = document.getElementById("code");
            var notification = document.querySelector(".notification");
            if (codeInput) {
                navigator.clipboard.writeText(codeInput.value).then(function() {
                    notification.style.display = "block";
                    setTimeout(function() {
                        notification.style.display = "none";
                        window.close();
                    }, 2000);
                }).catch(function(err) {
                    console.error("Could not copy text: ", err);
                });
            }
        }

        document.addEventListener("DOMContentLoaded", function() {
            copyCodeToClipboard();
            document.body.addEventListener("click", copyCodeToClipboard);
        });
    </script>
</body>
</html>