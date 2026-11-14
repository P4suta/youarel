import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.util.Arrays;
import org.apache.commons.io.FileUtils;

class SigningFiles {
    public static void main(String[] args) throws IOException {
        if (args.length != 2) {
            throw new IllegalArgumentException("Expected source and destination paths");
        }
        File source = new File(args[0]);
        File destination = new File(args[1]);
        FileUtils.copyFile(source, destination);
        if (!Arrays.equals(Files.readAllBytes(source.toPath()), Files.readAllBytes(destination.toPath()))) {
            throw new IOException("SDK file output changed the fixture");
        }
        System.out.println("Verified SDK file output.");
    }
}
