// Import Rocket macros and external crates.
#[macro_use] extern crate rocket;

use rocket::form::Form;
use rocket::fs::{TempFile, FileServer, relative};
use rocket::response::content::RawHtml;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::io::Write;
use std::fs;
use std::env;
use std::path::{Path, PathBuf};

/// Data structure representing a single test case.
/// Each test case includes a description, input string, and expected output.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct TestCase {
    description: String,
    input: String,
    expected_output: String,
}

/// Type alias for a mapping from question IDs to lists of test cases.
type TestCasesMap = HashMap<String, Vec<TestCase>>;

//
// Routes for Uploading and Testing Code
//

/// Form data structure for file uploads.
/// Includes the uploaded C file and the question identifier.
#[derive(FromForm)]
struct Upload<'r> {
    file: TempFile<'r>,
    question: String,
}

/// GET /
/// Returns the index page with a form for uploading C code.
/// The page also includes general website info about the autograder and a dynamic
/// description of the selected question.
#[get("/")]
async fn index() -> RawHtml<&'static str> {
    RawHtml(r##"
<!DOCTYPE html>
<html>
  <head>
    <meta charset="UTF-8">
    <title>Autograder</title>
    <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
    <link rel="icon" href="/static/favicon.ico" type="image/x-icon">
    <style>
      body { background-color: #f8f9fa; }
      .header { text-align: center; margin-top: 20px; }
      .info { text-align: center; margin-bottom: 30px; }
    </style>
  </head>
  <body>
    <div class="container mt-5">
      <div class="header">
        <h1>Autograder</h1>
        <p class="info">Welcome to the Autograder! Upload your C code to get instant feedback based on predefined test cases.</p>
      </div>
      <form action="/upload" method="post" enctype="multipart/form-data">
        <div class="mb-3">
          <label for="question" class="form-label">Select Question:</label>
          <select id="question" name="question" class="form-select">
            <option value="q1">Q1: Average Positive/Negative</option>
            <option value="q2">Q2: Tic Tac Toe</option>
            <option value="q3">Q3: Series Sum (Repeated 9s)</option>
            <option value="q4">Q4: Count Pos/Neg/Zero (Stop on Repeat)</option>
            <option value="q5">Q5: Min/Second/Third/Largest with Termination</option>
            <option value="q6">Q6: Vowel or Consonant Checker</option>
            <option value="q7">Q7: Prime Number Finder</option>
            <option value="q8">Q8: Right-Angle Triangle Pattern</option>
            <option value="q9">Q9: Sum of Even Natural Numbers</option>
            <option value="q10">Q10: Reverse a Number</option>
            <option value="q11">Q11: Taylor Series 1/(1-x)</option>
            <option value="q12">Q12: Bitwise Operations (Hex Input)</option>
            <option value="q13">Q13: Cosine Calculation (Taylor Series)</option>
            <option value="q14">Q14: Binary (Hex) to Decimal Conversion</option>
            <option value="q15">Q15: File Copy Scanner</option>
            <option value="q16">Q16: Extract Identifiers</option>
            <option value="q17">Q17: Uppercase Identifiers</option>
            <option value="q18">Q18: Recognize Operators</option>
            <option value="q19">Q19: Recognize Additional Operators</option>
            <option value="q20">Q20: Recognize Special Characters</option>
            <option value="q21">Q21: Integrated Scanner Program</option>
          </select>
        </div>
        <div class="mb-3">
          <p id="question-description" class="text-muted"></p>
        </div>
        <div class="mb-3">
          <label for="file" class="form-label">C File:</label>
          <input type="file" class="form-control" id="file" name="file" accept=".c">
        </div>
        <button type="submit" class="btn btn-primary">Submit</button>
      </form>
    </div>
    <script>
      const descriptions = {
         'q1': 'Compute average of positive and negative numbers from 15 decimal inputs.',
         'q2': 'Determine the Tic Tac Toe game outcome (win, draw, in progress, invalid board).',
         'q3': 'Compute the sum of the series: 9 + 99 + 999 + ... for n terms.',
         'q4': 'Count positive, negative, and zero values until the same value is entered consecutively.',
         'q5': 'Find the smallest, second smallest, third smallest, and largest values with termination when the largest remains unchanged for n iterations.',
         'q6': 'Repeatedly check if an input character is a vowel or a consonant (ends on "#").',
         'q7': 'Find and display all prime numbers within a given range.',
         'q8': 'Display a right-angle triangle pattern based on the number of rows provided.',
         'q9': 'Display even natural numbers for n terms and compute their sum.',
         'q10': 'Reverse the digits of a given number.',
         'q11': 'Compute 1/(1-x) using Taylor series expansion until the precision threshold is met.',
         'q12': 'Perform a bitwise operation (AND, OR, XOR) on two hexadecimal inputs.',
         'q13': 'Calculate the cosine of a given angle (in degrees) using Taylor series.',
         'q14': 'Convert a binary number provided as a hexadecimal input into its decimal equivalent.',
         'q15': 'Copy an input text file exactly to an output file.',
         'q16': 'Scan a text file and extract identifiers.',
         'q17': 'Scan a text file and output all identifiers in uppercase.',
         'q18': 'Scan a text file and recognize basic operators (+, -, *, /, %).',
         'q19': 'Extend operator recognition to include increment, decrement, assignment, and compound operators.',
         'q20': 'Recognize special characters (dot, comma, semicolon, colon) in a text file.',
         'q21': 'An integrated scanner program that combines identifier, operator, and special character recognition.'
      };
      const select = document.getElementById('question');
      const descElem = document.getElementById('question-description');
      function updateDescription() {
         const selected = select.value;
         descElem.textContent = descriptions[selected] || '';
      }
      select.addEventListener('change', updateDescription);
      // Initialize on load
      updateDescription();
    </script>
  </body>
</html>
"##)
}

/// POST /upload
/// Handles the file upload, compiles the C code, loads test cases, and runs them
/// inside a sandboxed environment using NSJail. Displays a test summary along with
/// individual test results.
#[post("/upload", data = "<form>")]
async fn upload(mut form: Form<Upload<'_>>) -> RawHtml<String> {
    use uuid::Uuid;

    // Check if a file was uploaded.
    if form.file.name().is_none() {
        return RawHtml("<h2>No file uploaded. Try again with a valid .c file</h2>".to_string());
    }

    // Build an absolute path for the temporary directory.
    let cwd = env::current_dir().expect("Failed to get current directory");
    let temp_dir: PathBuf = cwd.join("tempfiles");

    // Create the tempfiles directory if it doesn't exist.
    if let Err(e) = fs::create_dir_all(&temp_dir) {
        return RawHtml(format!("<h2>Error creating temp directory: {}</h2>", e));
    }

    // Generate unique file names using Uuid.
    let unique_id = Uuid::new_v4().to_string();
    let tmp_path = temp_dir.join(format!("{}.c", unique_id));
    let exe_path = temp_dir.join(&unique_id);

    // Save the uploaded C file to disk.
    if let Err(e) = form.file.persist_to(&tmp_path).await {
        return RawHtml(format!("<h2>Error saving file: {}</h2>", e));
    }

    // Compile the C file using gcc.
    let compile = Command::new("gcc")
        .arg("-o")
        .arg(&exe_path)
        .arg(&tmp_path)
        .output();

    // Handle compilation errors.
    let compile_output = match compile {
        Ok(output) => output,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            return RawHtml(format!("<h2>Compilation failed: {}</h2>", e));
        }
    };

    if !compile_output.status.success() {
        let err_msg = String::from_utf8_lossy(&compile_output.stderr);
        let _ = fs::remove_file(&tmp_path);
        return RawHtml(format!("<h2>Compilation errors:</h2><pre>{}</pre>", err_msg));
    }

    // Set executable permissions explicitly.
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755)) {
        eprintln!("Error setting permissions on executable: {}", e);
    }

    // Verify that the executable exists.
    let exe_path_str = exe_path.to_string_lossy().into_owned();
    if !Path::new(&exe_path).exists() {
        eprintln!("Executable not found at: {}", exe_path_str);
        return RawHtml("<h2>Internal error: compiled executable not found.</h2>".to_string());
    }

    // Load test cases from the external JSON file.
    let test_cases_data = match fs::read_to_string("test_cases.json") {
        Ok(data) => data,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>Error reading test cases file: {}</h2>", e));
        }
    };

    let test_cases_map: TestCasesMap = match serde_json::from_str(&test_cases_data) {
        Ok(tc) => tc,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>Error parsing test cases file: {}</h2>", e));
        }
    };

    let selected_question = &form.question;
    let cases = match test_cases_map.get(selected_question) {
        Some(c) => c,
        None => {
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(&exe_path);
            return RawHtml(format!("<h2>No test cases found for question {}</h2>", selected_question));
        }
    };

    let mut results = Vec::new();

    // Loop through each test case.
    for case in cases {
        // Check again that the executable exists.
        if !Path::new(&exe_path).exists() {
            eprintln!("Executable not found at: {}", exe_path_str);
            return RawHtml("<h2>Internal error: compiled executable not found.</h2>".to_string());
        }

        // Use NSJail to run the executable.
        let mut child = match Command::new("nsjail")
            .args(&[
                "--mode=exec",
                "--disable_clone_newuser",
                "--bindmount", "/app/tempfiles:/app/tempfiles",
                "--bindmount", "/lib:/lib",
                "--bindmount", "/usr/lib:/usr/lib",
                "--", &exe_path_str,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn() {
                Ok(child) => child,
                Err(e) => {
                    results.push((case.description.clone(), false, format!("Error running the program with NSJail: {}", e)));
                    continue;
                }
            };

        // Write the test case input to the program's stdin.
        {
            let child_stdin = child.stdin.as_mut().expect("Failed to open stdin");
            if let Err(e) = child_stdin.write_all(case.input.as_bytes()) {
                results.push((case.description.clone(), false, format!("Error writing to stdin: {}", e)));
                continue;
            }
        }

        // Wait for the program to finish and capture its output.
        let run_output = match child.wait_with_output() {
            Ok(output) => output,
            Err(e) => {
                results.push((case.description.clone(), false, format!("Error waiting for output: {}", e)));
                continue;
            }
        };

        let actual_output = String::from_utf8_lossy(&run_output.stdout).trim().to_string();
        let expected_substring = case.expected_output.trim();
        let passed = actual_output.contains(expected_substring);
        let result_text = format!("Input: {}\nExpected to contain: {}\nGot: {}", case.input, expected_substring, actual_output);
        results.push((case.description.clone(), passed, result_text));
    }

    // Clean up temporary files after processing all test cases.
    let _ = fs::remove_file(&tmp_path);
    let _ = fs::remove_file(&exe_path);

    // Calculate test summary.
let total_tests = results.len();
let passed_tests = results.iter().filter(|(_, passed, _)| *passed).count();
let passing_percentage = if total_tests > 0 {
    passed_tests as f64 / total_tests as f64 * 100.0
} else {
    0.0
};
let summary_html = format!(
    "<div class='alert alert-info' style='font-family: \"Segoe UI\", sans-serif;'>
       <h2>Test Summary</h2>
       <p>Passed {}/{} test cases ({:.2}%)</p>
     </div>",
    passed_tests, total_tests, passing_percentage
);

// Build the HTML output with improved styling.
let mut results_html = String::from("<h1 style='font-family: \"Segoe UI\", sans-serif;'>Test Results</h1>");
results_html.push_str(&summary_html);
results_html.push_str("<div id='results'>");

for (i, (desc, passed, details)) in results.into_iter().enumerate() {
    let bg_class = if passed { "bg-success" } else { "bg-danger" };
    // For failed tests, wrap the details in a diff span to highlight the error.
    let detail_markup = if passed {
        details
    } else {
        format!("<span class='diff'>{}</span>", details)
    };
    results_html.push_str(&format!(
        "<div class='list-group-item {} text-white test-result' style='display:none; font-family: \"Segoe UI\", sans-serif; padding: 10px; border-radius: 5px; margin-bottom: 5px;' data-delay='{}'>
           <strong>{}</strong>: {}
           <pre style='background-color: #f1f1f1; color: #333; padding: 10px; border-radius: 5px; font-family: \"Courier New\", monospace;'>{}</pre>
         </div>",
        bg_class,
        i * 500,
        desc,
        if passed { "Passed" } else { "Failed" },
        detail_markup
    ));
}
results_html.push_str("</div><a href='/' class='btn btn-secondary mt-3' style='font-family: \"Segoe UI\", sans-serif;'>Upload another file</a>");

let script = r#"
<script>
  window.addEventListener('DOMContentLoaded', () => {
    const results = document.querySelectorAll('.test-result');
    results.forEach((result, index) => {
      setTimeout(() => {
        result.style.display = 'block';
        result.style.opacity = 0;
        let op = 0;
        const timer = setInterval(() => {
          if (op >= 1) clearInterval(timer);
          result.style.opacity = op;
          op += 0.1;
        }, 30);
      }, index * 500);
    });
  });
</script>
"#;

let full_html = format!(r#"
<!DOCTYPE html>
<html>
  <head>
    <meta charset="UTF-8">
    <title>Execution Result</title>
    <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
    <style>
      body {{
          background-color: #f8f9fa;
          font-family: "Segoe UI", sans-serif;
      }}
      .container {{
          margin-top: 30px;
      }}
      .test-result {{
          margin-bottom: 10px;
          padding: 10px;
          border-radius: 5px;
      }}
      pre {{
          background-color: #f1f1f1;
          padding: 10px;
          border-radius: 5px;
          font-family: "Courier New", monospace;
      }}
      .diff {{
          background-color: #ffdddd;
          font-weight: bold;
      }}
    </style>
  </head>
  <body>
    <div class="container">
      {}
    </div>
    {}
  </body>
</html>
"#, results_html, script);

RawHtml(full_html)
}

//
// Admin Panel Routes
//

/// GET /admin
/// Returns the admin login page with enhanced styling and autograder info.
#[get("/admin")]
async fn admin_login_page() -> RawHtml<String> {
    RawHtml(r#"
    <!DOCTYPE html>
    <html>
      <head>
        <meta charset="UTF-8">
        <title>Admin Login - Autograder</title>
        <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
        <style>
          body { background-color: #f8f9fa; }
          .header { text-align: center; margin-top: 20px; }
        </style>
      </head>
      <body>
        <div class="container mt-5">
          <div class="header">
            <h1>Autograder Admin Panel</h1>
            <p>Please login to manage test cases.</p>
          </div>
          <form action="/admin" method="post">
            <div class="mb-3">
              <label for="password" class="form-label">Password:</label>
              <input type="password" id="password" name="password" class="form-control">
            </div>
            <button type="submit" class="btn btn-primary">Login</button>
          </form>
        </div>
      </body>
    </html>
    "#.to_string())
}

/// Data structure representing admin login credentials.
#[derive(rocket::form::FromForm)]
struct AdminLogin {
    password: String,
}

/// POST /admin
/// Processes the admin login and, if successful, displays the admin panel.
#[post("/admin", data = "<form>")]
async fn admin_login(form: Form<AdminLogin>) -> RawHtml<String> {
    let admin_password = "secret";
    if form.password != admin_password {
        return RawHtml("<h2>Invalid password.</h2><a href='/admin'>Try again</a>".to_string());
    }
    let html = r#"
       <!DOCTYPE html>
       <html>
         <head>
           <meta charset="UTF-8">
           <title>Admin Panel - Autograder</title>
           <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
           <style>
             body { background-color: #f8f9fa; }
             .header { text-align: center; margin-top: 20px; }
           </style>
         </head>
         <body>
           <div class="container mt-5">
             <div class="header">
               <h1>Autograder Admin Panel</h1>
               <p>Select a question to edit its test cases.</p>
             </div>
            <ul class="list-group">
              <li class="list-group-item"><a href="/admin/edit?question=q1">Edit Q1 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q2">Edit Q2 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q3">Edit Q3 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q4">Edit Q4 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q5">Edit Q5 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q6">Edit Q6 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q7">Edit Q7 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q8">Edit Q8 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q9">Edit Q9 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q10">Edit Q10 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q11">Edit Q11 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q12">Edit Q12 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q13">Edit Q13 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q14">Edit Q14 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q15">Edit Q15 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q16">Edit Q16 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q17">Edit Q17 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q18">Edit Q18 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q19">Edit Q19 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q20">Edit Q20 Test Cases</a></li>
              <li class="list-group-item"><a href="/admin/edit?question=q21">Edit Q21 Test Cases</a></li>
            </ul>
           </div>
         </body>
       </html>
    "#;
    RawHtml(html.to_string())
}

/// GET /admin/edit
/// Returns a page for editing test cases for a given question with improved styling.
#[get("/admin/edit?<question>")]
async fn admin_edit_page(question: Option<String>) -> RawHtml<String> {
    let q = question.unwrap_or_else(|| "q1".to_string());
    let content = fs::read_to_string("test_cases.json").unwrap_or_else(|_| "{}".to_string());
    let mut test_cases_map: TestCasesMap = serde_json::from_str(&content).unwrap_or_else(|_| HashMap::new());
    let cases = test_cases_map.entry(q.clone()).or_insert(Vec::new());

    // You may wish to customize the question description here as well.
    let question_desc = match q.as_str() {
      "q1"  => "Compute average of positive and negative numbers from 15 decimal inputs.",
      "q2"  => "Determine the Tic Tac Toe game outcome (win, draw, in progress, invalid board).",
      "q3"  => "Compute the sum of the series: 9 + 99 + 999 + ... for n terms.",
      "q4"  => "Count positive, negative, and zero values until the same value is entered consecutively.",
      "q5"  => "Find the smallest, second smallest, third smallest, and largest values with termination when the largest remains unchanged for n iterations.",
      "q6"  => "Repeatedly check if an input character is a vowel or a consonant (ends on \"#\").",
      "q7"  => "Find and display all prime numbers within a given range.",
      "q8"  => "Display a right-angle triangle pattern based on the number of rows provided.",
      "q9"  => "Display even natural numbers for n terms and compute their sum.",
      "q10" => "Reverse the digits of a given number.",
      "q11" => "Compute 1/(1-x) using Taylor series expansion until the precision threshold is met.",
      "q12" => "Perform a bitwise operation (AND, OR, XOR) on two hexadecimal inputs.",
      "q13" => "Calculate the cosine of a given angle (in degrees) using Taylor series.",
      "q14" => "Convert a binary number provided as a hexadecimal input into its decimal equivalent.",
      "q15" => "Copy an input text file exactly to an output file.",
      "q16" => "Scan a text file and extract identifiers.",
      "q17" => "Scan a text file and output all identifiers in uppercase.",
      "q18" => "Scan a text file and recognize basic operators (+, -, *, /, %).",
      "q19" => "Extend operator recognition to include increment, decrement, assignment, and compound operators.",
      "q20" => "Recognize special characters (dot, comma, semicolon, colon) in a text file.",
      "q21" => "An integrated scanner program that combines identifier, operator, and special character recognition.",
      _     => ""
  };

    let mut form_html = format!(r#"
       <!DOCTYPE html>
       <html>
         <head>
           <meta charset="UTF-8">
           <title>Edit Test Cases for {}</title>
           <link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.0/dist/css/bootstrap.min.css" rel="stylesheet">
           <style>
             .test-case {{
               margin-bottom: 20px; 
               padding: 10px; 
               border: 1px solid #ccc; 
               position: relative;
               background-color: #ffffff;
             }}
             .remove-btn {{
               position: absolute;
               top: 10px;
               right: 10px;
             }}
             body {{ background-color: #f8f9fa; }}
           </style>
         </head>
         <body>
           <div class="container mt-5">
             <h1>Edit Test Cases for {} ({})</h1>
             <p class="mb-4">Modify the test cases below or add new ones as needed.</p>
             <form id="test-cases-form" action="/admin/edit" method="post">
               <input type="hidden" name="question" value="{}">
    "#, q, q, question_desc, q);

    for case in cases {
        form_html.push_str(&format!(r#"
           <div class="test-case">
             <button type="button" class="btn btn-danger btn-sm remove-btn" onclick="removeTestCase(this)">X</button>
             <div class="mb-3">
               <label>Description:</label>
               <input type="text" class="form-control" name="desc" value="{}">
             </div>
             <div class="mb-3">
               <label>Input:</label>
               <textarea class="form-control" name="inp" rows="3">{}</textarea>
             </div>
             <div class="mb-3">
               <label>Expected Output:</label>
               <textarea class="form-control" name="exp" rows="3">{}</textarea>
             </div>
           </div>
        "#,
        htmlescape::encode_minimal(&case.description),
        htmlescape::encode_minimal(&case.input),
        htmlescape::encode_minimal(&case.expected_output)));
    }

    form_html.push_str(r#"
               <div id="new-test-case-container"></div>
               <button type="button" class="btn btn-secondary" onclick="addTestCase()">Add Test Case</button>
               <button type="submit" class="btn btn-primary">Save Changes</button>
             </form>
             <a href="/admin" class="btn btn-secondary mt-3">Back to Admin Panel</a>
           </div>
           <script>
             function removeTestCase(button) {
               button.parentElement.remove();
             }
             function addTestCase() {
               const container = document.getElementById("new-test-case-container");
               const div = document.createElement("div");
               div.className = "test-case";
               div.innerHTML = `
                 <button type="button" class="btn btn-danger btn-sm remove-btn" onclick="removeTestCase(this)">Remove</button>
                 <div class="mb-3">
                   <label>Description:</label>
                   <input type="text" class="form-control" name="desc" value="">
                 </div>
                 <div class="mb-3">
                   <label>Input:</label>
                   <textarea class="form-control" name="inp" rows="3"></textarea>
                 </div>
                 <div class="mb-3">
                   <label>Expected Output:</label>
                   <textarea class="form-control" name="exp" rows="3"></textarea>
                 </div>
               `;
               container.appendChild(div);
             }
           </script>
         </body>
       </html>
    "#);
    RawHtml(form_html)
}

/// Data structure for the form used to update test cases.
#[derive(rocket::form::FromForm)]
struct AdminEditForm {
    question: String,
    desc: Vec<String>,
    inp: Vec<String>,
    exp: Vec<String>,
}

/// POST /admin/edit
/// Updates the test cases for a given question and writes them back to the JSON file.
#[post("/admin/edit", data = "<form>")]
async fn admin_edit_update(form: Form<AdminEditForm>) -> RawHtml<String> {
    let q = &form.question;
    let content = fs::read_to_string("test_cases.json").unwrap_or_else(|_| "{}".to_string());
    let mut test_cases_map: TestCasesMap = serde_json::from_str(&content).unwrap_or_else(|_| HashMap::new());
    let mut new_cases = Vec::new();
    let n = form.desc.len().min(form.inp.len()).min(form.exp.len());
    for i in 0..n {
        new_cases.push(TestCase {
            description: form.desc[i].clone(),
            input: form.inp[i].clone(),
            expected_output: form.exp[i].clone(),
        });
    }
    test_cases_map.insert(q.clone(), new_cases);
    let new_content = serde_json::to_string_pretty(&test_cases_map).unwrap_or_else(|_| "{}".to_string());
    if let Err(e) = fs::write("test_cases.json", new_content) {
        return RawHtml(format!("<h2>Error updating test cases: {}</h2>", e));
    }
    
    RawHtml(format!("<h2>Test cases for {} updated successfully.</h2><a href='/admin'>Back to Admin Panel</a>", q))
}

//
// Launch the Application
//

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![
            index, 
            upload, 
            admin_login_page, 
            admin_login, 
            admin_edit_page, 
            admin_edit_update
        ])
        .mount("/static", FileServer::from(relative!("static")))
}